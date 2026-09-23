//! The Design schema. Mirrors docs/09-design-format.md.
//!
//! Every struct is `#[serde(default)]`: a Design only needs to state what it
//! changes, and a file missing a section still loads.

use crate::Color;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Design {
    pub meta: Meta,
    pub background: Background,
    pub notes: NoteStyle,
    pub keyboard: Keyboard,
    pub particles: Vec<Emitter>,
    pub post: Post,
    pub camera: Camera,
    pub layout: Layout,
}

impl Default for Design {
    /// The reference look. Kept in sync with `designs/ember-classic.toml` by
    /// a test, so the code default and the shipped file never disagree.
    fn default() -> Self {
        Self {
            meta: Meta::default(),
            background: Background::default(),
            notes: NoteStyle::default(),
            keyboard: Keyboard::default(),
            particles: vec![Emitter::default(), Emitter::hold_default()],
            post: Post::default(),
            camera: Camera::default(),
            layout: Layout::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub name: String,
    pub author: String,
    /// Schema version, not design version.
    pub version: u32,
    pub description: String,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            author: String::new(),
            version: SCHEMA_VERSION,
            description: String::new(),
        }
    }
}

// ── Background ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundKind {
    Solid,
    Gradient,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// Cover the frame, cropping the image.
    Fill,
    /// Whole image visible, letterboxed.
    Fit,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    pub at: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    #[serde(rename = "type")]
    pub kind: BackgroundKind,
    /// Used by `solid`.
    pub color: Color,
    /// Used by `gradient`. Interpolated in linear light.
    pub stops: Vec<Stop>,
    /// Degrees; 90 runs bottom to top.
    pub angle: f32,
    pub radial: bool,
    /// Used by `image`: path relative to the Design bundle.
    pub image: String,
    pub fit: ImageFit,
    pub tint: Color,
    pub reflection: Reflection,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            kind: BackgroundKind::Gradient,
            color: Color::rgb(0x05, 0x06, 0x0a),
            stops: vec![
                Stop { at: 0.0, color: Color::rgb(0x0b, 0x07, 0x06) },
                Stop { at: 1.0, color: Color::rgb(0x03, 0x04, 0x08) },
            ],
            angle: 90.0,
            radial: false,
            image: String::new(),
            fit: ImageFit::Fill,
            tint: Color::WHITE,
            reflection: Reflection::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reflection {
    pub enabled: bool,
    pub opacity: f32,
    /// Blur radius in pixels at 1080p.
    pub blur: f32,
    /// 0 fades immediately below the keys, 1 reaches the bottom of the frame.
    pub fade: f32,
}

impl Default for Reflection {
    fn default() -> Self {
        Self { enabled: false, opacity: 0.25, blur: 4.0, fade: 0.7 }
    }
}

// ── Notes ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientDir {
    None,
    /// Bright at the leading (bottom) edge, fading along the note.
    Along,
    /// Bright in the middle, darker at the sides.
    Across,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSource {
    Track,
    Channel,
    /// Left of the split point uses palette[0], right uses palette[1].
    Hand,
    /// One palette entry per semitone of the octave.
    PitchClass,
    Fixed,
    /// Palette spread from soft (first) to loud (last).
    Velocity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteColor {
    pub source: ColorSource,
    pub palette: Vec<Color>,
    /// How much MIDI velocity scales brightness, 0..1.
    pub velocity_brightness: f32,
    /// Hand split pitch; `None` picks one from the music.
    pub split: Option<u8>,
    /// Multiply black-key notes by this, so they read as black keys.
    pub black_key_shade: f32,
}

impl Default for NoteColor {
    fn default() -> Self {
        Self {
            source: ColorSource::Track,
            palette: vec![
                Color::rgb(0xff, 0x7a, 0x2e),
                Color::rgb(0xff, 0xc8, 0x6b),
                Color::rgb(0x4f, 0xc3, 0xc0),
                Color::rgb(0xe8, 0x4a, 0x6f),
                Color::rgb(0x8e, 0x7c, 0xff),
            ],
            velocity_brightness: 0.4,
            split: None,
            black_key_shade: 0.75,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SustainTail {
    /// Draw the pedal's extra ring time as a dimmer tail behind the note.
    pub enabled: bool,
    pub opacity: f32,
}

impl Default for SustainTail {
    fn default() -> Self {
        Self { enabled: false, opacity: 0.14 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteStyle {
    /// Pixels at 1080p. All pixel sizes scale with output height.
    pub corner_radius: f32,
    pub border_width: f32,
    pub border_color: Color,
    pub opacity: f32,
    /// HDR multiplier: values above 1 bloom.
    pub intensity: f32,
    pub gradient: GradientDir,
    pub gradient_falloff: f32,
    /// Horizontal gap between a note and its key edges, pixels at 1080p.
    pub margin: f32,
    /// Brighten a note while it is sounding.
    pub active_boost: f32,
    pub color: NoteColor,
    pub sustain_tail: SustainTail,
}

impl Default for NoteStyle {
    fn default() -> Self {
        Self {
            corner_radius: 4.0,
            border_width: 1.5,
            border_color: Color::rgba(0xff, 0xf2, 0xe0, 0x70),
            opacity: 1.0,
            intensity: 1.6,
            gradient: GradientDir::Along,
            gradient_falloff: 0.35,
            margin: 1.0,
            active_boost: 1.35,
            color: NoteColor::default(),
            sustain_tail: SustainTail::default(),
        }
    }
}

// ── Keyboard ────────────────────────────────────────────────────────────────

/// Which keys to draw. Written as `"auto"`, `"88"`, `"76"`, `"61"`, `"49"`,
/// or an explicit `"21-108"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum KeyRange {
    /// Fit the music, rounded out to whole octaves' natural boundaries.
    Auto,
    Fixed(u8, u8),
}

impl TryFrom<String> for KeyRange {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let s = s.trim();
        let r = match s {
            "auto" => return Ok(Self::Auto),
            "88" => (21, 108),
            "76" => (28, 103),
            "61" => (36, 96),
            "49" => (36, 84),
            "128" => (0, 127),
            _ => {
                let (lo, hi) =
                    s.split_once('-').ok_or_else(|| format!("invalid key range {s:?}"))?;
                let lo: u8 = lo.trim().parse().map_err(|_| format!("invalid key range {s:?}"))?;
                let hi: u8 = hi.trim().parse().map_err(|_| format!("invalid key range {s:?}"))?;
                if lo >= hi || hi > 127 {
                    return Err(format!("invalid key range {s:?}: need low < high <= 127"));
                }
                (lo, hi)
            }
        };
        Ok(Self::Fixed(r.0, r.1))
    }
}

impl From<KeyRange> for String {
    fn from(r: KeyRange) -> String {
        match r {
            KeyRange::Auto => "auto".into(),
            KeyRange::Fixed(21, 108) => "88".into(),
            KeyRange::Fixed(lo, hi) => format!("{lo}-{hi}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pressed {
    /// Pressed keys take the color of the note pressing them.
    pub tint_from_note: bool,
    /// 0 keeps the key color, 1 replaces it with the note color.
    pub tint_amount: f32,
    pub glow_intensity: f32,
    /// Pixels at 1080p.
    pub glow_radius: f32,
    /// How far a pressed key visibly moves, pixels at 1080p.
    pub depress_pixels: f32,
}

impl Default for Pressed {
    fn default() -> Self {
        Self {
            tint_from_note: true,
            tint_amount: 0.85,
            glow_intensity: 1.8,
            glow_radius: 24.0,
            depress_pixels: 2.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keyboard {
    pub range: KeyRange,
    /// Fraction of the frame height.
    pub height: f32,
    pub white_key_color: Color,
    pub black_key_color: Color,
    pub black_width_ratio: f32,
    pub black_length_ratio: f32,
    pub separator_width: f32,
    pub separator_color: Color,
    /// A thin bright line where notes meet the keys.
    pub strike_line: StrikeLine,
    pub pressed: Pressed,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            range: KeyRange::Fixed(21, 108),
            height: 0.16,
            white_key_color: Color::rgb(0xe9, 0xe6, 0xe1),
            black_key_color: Color::rgb(0x12, 0x11, 0x12),
            black_width_ratio: 0.58,
            black_length_ratio: 0.63,
            separator_width: 1.0,
            separator_color: Color::rgb(0x55, 0x52, 0x50),
            strike_line: StrikeLine::default(),
            pressed: Pressed::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrikeLine {
    pub enabled: bool,
    pub color: Color,
    pub width: f32,
    pub intensity: f32,
}

impl Default for StrikeLine {
    fn default() -> Self {
        Self { enabled: true, color: Color::rgb(0xff, 0x9a, 0x4a), width: 2.0, intensity: 1.2 }
    }
}

// ── Particles ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmitEvent {
    /// A burst when the note reaches the keys.
    NoteHit,
    /// A stream for as long as the note is held.
    NoteHold,
    /// A burst when the note ends.
    NoteRelease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    Constant,
    /// 1 → 0 linearly.
    Shrink,
    /// 0 → 1.
    Grow,
    /// Quick rise, long fall.
    Pulse,
    /// Full, then fading over the last half of life.
    FadeOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleColor {
    Note,
    Palette,
    Fixed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Emitter {
    pub enabled: bool,
    pub event: EmitEvent,
    /// Particles per burst (`note_hit`, `note_release`).
    pub count: u32,
    /// Particles per second (`note_hold`).
    pub rate: f32,
    /// Seconds.
    pub lifetime: f32,
    /// Fraction of lifetime, 0..1.
    pub lifetime_variance: f32,
    /// Pixels per second at 1080p.
    pub speed: f32,
    pub speed_variance: f32,
    /// Degrees; 90 is straight up.
    pub direction_degrees: f32,
    /// Full cone width in degrees.
    pub spread_degrees: f32,
    /// Pixels per second squared at 1080p; y is up.
    pub gravity: [f32; 2],
    /// Linear drag coefficient, per second.
    pub drag: f32,
    /// Radius, pixels at 1080p.
    pub size: f32,
    pub size_variance: f32,
    pub size_curve: Curve,
    pub opacity_curve: Curve,
    /// Drift amplitude, pixels per second at 1080p.
    pub turbulence: f32,
    pub color_source: ParticleColor,
    pub color: Color,
    /// Blend toward white-hot at birth, 0..1.
    pub white_hot: f32,
    pub intensity: f32,
    /// How much MIDI velocity scales count, speed, and size, 0..1.
    pub velocity_response: f32,
}

impl Default for Emitter {
    fn default() -> Self {
        Self {
            enabled: true,
            event: EmitEvent::NoteHit,
            count: 36,
            rate: 12.0,
            lifetime: 1.3,
            lifetime_variance: 0.4,
            speed: 240.0,
            speed_variance: 0.6,
            direction_degrees: 90.0,
            spread_degrees: 80.0,
            gravity: [0.0, -160.0],
            drag: 1.2,
            size: 2.6,
            size_variance: 0.5,
            size_curve: Curve::Shrink,
            opacity_curve: Curve::FadeOut,
            turbulence: 26.0,
            color_source: ParticleColor::Note,
            color: Color::rgb(0xff, 0xb0, 0x60),
            white_hot: 0.45,
            intensity: 3.0,
            velocity_response: 0.7,
        }
    }
}

impl Emitter {
    pub fn hold_default() -> Self {
        Self {
            event: EmitEvent::NoteHold,
            count: 0,
            rate: 10.0,
            lifetime: 2.2,
            speed: 50.0,
            speed_variance: 0.5,
            spread_degrees: 30.0,
            gravity: [0.0, 50.0],
            drag: 0.6,
            size: 1.8,
            turbulence: 36.0,
            white_hot: 0.2,
            intensity: 2.2,
            ..Self::default()
        }
    }

    /// Upper bound on particles one note can have alive at once, used to size
    /// the per-note instance stride.
    pub fn max_alive_per_note(&self) -> u32 {
        if !self.enabled {
            return 0;
        }
        match self.event {
            EmitEvent::NoteHit | EmitEvent::NoteRelease => self.count,
            EmitEvent::NoteHold => (self.rate * self.lifetime).ceil() as u32 + 1,
        }
    }
}

// ── Post ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Post {
    pub bloom: Bloom,
    pub tonemap: Tonemap,
    pub vignette: Vignette,
    pub grain: Grain,
    pub chromatic_aberration: ChromaticAberration,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bloom {
    pub enabled: bool,
    pub threshold: f32,
    pub soft_knee: f32,
    pub intensity: f32,
    /// Blend between tight (0) and wide (1) falloff.
    pub radius: f32,
    pub tint: Color,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 0.8,
            soft_knee: 0.5,
            intensity: 0.9,
            radius: 0.75,
            tint: Color::rgb(0xff, 0xe4, 0xc8),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TonemapOp {
    Agx,
    Aces,
    Reinhard,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tonemap {
    pub operator: TonemapOp,
    /// Stops of exposure; 0 is neutral.
    pub exposure: f32,
    /// 1 is neutral.
    pub saturation: f32,
}

impl Default for Tonemap {
    fn default() -> Self {
        Self { operator: TonemapOp::Agx, exposure: 0.0, saturation: 1.1 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vignette {
    pub enabled: bool,
    pub amount: f32,
    pub smoothness: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self { enabled: true, amount: 0.35, smoothness: 0.5 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grain {
    pub enabled: bool,
    pub amount: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self { enabled: true, amount: 0.025 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChromaticAberration {
    pub enabled: bool,
    pub amount: f32,
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self { enabled: false, amount: 0.002 }
    }
}

// ── Camera ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    pub zoom: f32,
    pub shake: Shake,
    pub drift: Drift,
}

impl Default for Camera {
    fn default() -> Self {
        Self { zoom: 1.0, shake: Shake::default(), drift: Drift::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shake {
    pub enabled: bool,
    /// Peak offset, pixels at 1080p.
    pub amount: f32,
    /// Decay rate per second.
    pub decay: f32,
    pub velocity_response: f32,
}

impl Default for Shake {
    fn default() -> Self {
        Self { enabled: false, amount: 2.5, decay: 9.0, velocity_response: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Drift {
    pub enabled: bool,
    pub amount: f32,
    pub speed: f32,
}

impl Default for Drift {
    fn default() -> Self {
        Self { enabled: false, amount: 6.0, speed: 0.08 }
    }
}

// ── Layout ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Notes fall toward a keyboard at the bottom.
    Down,
    /// Notes rise from a keyboard at the bottom (live-mode style).
    Up,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Seconds of the future visible above the keys.
    pub lookahead: f32,
    pub direction: Direction,
}

impl Default for Layout {
    fn default() -> Self {
        Self { lookahead: 2.6, direction: Direction::Down }
    }
}

impl Design {
    /// Clamp every value to a range the renderer handles, so a hand-edited
    /// Design with a typo produces a strange look rather than a crash or a
    /// frozen GPU. Returns a note for each value it had to change.
    pub fn sanitize(&mut self) -> Vec<String> {
        fn clamp(out: &mut Vec<String>, name: &str, v: &mut f32, lo: f32, hi: f32) {
            let c = if v.is_finite() { v.clamp(lo, hi) } else { lo };
            if c != *v {
                out.push(format!("{name} = {v} is out of range; using {c}"));
                *v = c;
            }
        }
        let mut notes = Vec::new();
        clamp(&mut notes, "layout.lookahead", &mut self.layout.lookahead, 0.2, 30.0);
        clamp(&mut notes, "keyboard.height", &mut self.keyboard.height, 0.0, 0.6);
        clamp(
            &mut notes,
            "keyboard.black_width_ratio",
            &mut self.keyboard.black_width_ratio,
            0.2,
            1.0,
        );
        clamp(
            &mut notes,
            "keyboard.black_length_ratio",
            &mut self.keyboard.black_length_ratio,
            0.2,
            1.0,
        );
        clamp(&mut notes, "notes.opacity", &mut self.notes.opacity, 0.0, 1.0);
        clamp(&mut notes, "notes.intensity", &mut self.notes.intensity, 0.0, 50.0);
        clamp(&mut notes, "notes.corner_radius", &mut self.notes.corner_radius, 0.0, 200.0);
        clamp(&mut notes, "camera.zoom", &mut self.camera.zoom, 0.1, 10.0);
        clamp(&mut notes, "post.bloom.intensity", &mut self.post.bloom.intensity, 0.0, 10.0);
        clamp(&mut notes, "post.bloom.radius", &mut self.post.bloom.radius, 0.0, 1.0);
        clamp(&mut notes, "post.tonemap.exposure", &mut self.post.tonemap.exposure, -10.0, 10.0);
        for (i, e) in self.particles.iter_mut().enumerate() {
            clamp(&mut notes, &format!("particles[{i}].lifetime"), &mut e.lifetime, 0.01, 20.0);
            clamp(&mut notes, &format!("particles[{i}].rate"), &mut e.rate, 0.0, 500.0);
            clamp(&mut notes, &format!("particles[{i}].drag"), &mut e.drag, 0.0, 100.0);
            clamp(
                &mut notes,
                &format!("particles[{i}].lifetime_variance"),
                &mut e.lifetime_variance,
                0.0,
                0.95,
            );
            if e.count > MAX_BURST {
                notes.push(format!(
                    "particles[{i}].count = {} is above {MAX_BURST}; capped",
                    e.count
                ));
                e.count = MAX_BURST;
            }
        }
        if self.particles.len() > MAX_EMITTERS {
            notes.push(format!("only the first {MAX_EMITTERS} particle emitters are used"));
            self.particles.truncate(MAX_EMITTERS);
        }
        if self.notes.color.palette.is_empty() {
            self.notes.color.palette.push(Color::rgb(0xff, 0x7a, 0x2e));
        }
        notes
    }
}

/// Particles per burst. Bounds GPU work per struck note.
pub const MAX_BURST: u32 = 512;
pub const MAX_EMITTERS: usize = 4;
