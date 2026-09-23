//! GPU-side mirrors of the WGSL structs in `shaders/`. Field order and sizes
//! must match exactly; everything is packed into `vec4`s so WGSL's alignment
//! rules can't silently shift a field.

use crate::layout::KeyLayout;
use bytemuck::{Pod, Zeroable};
use pv_design::{
    BackgroundKind, ColorSource, Curve, Design, EmitEvent, GradientDir, ImageFit, ParticleColor,
    TonemapOp,
};

pub const MAX_STOPS: usize = 8;
pub const MAX_PALETTE: usize = 16;
pub const MAX_TRACKS: usize = 256;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub size: [f32; 4],
    pub view: [f32; 4],
    pub layout: [f32; 4],
    pub camera: [f32; 4],
    pub keys: [u32; 4],
    pub note_a: [f32; 4],
    pub note_b: [f32; 4],
    pub note_c: [f32; 4],
    pub border_color: [f32; 4],
    pub white_key: [f32; 4],
    pub black_key: [f32; 4],
    pub separator: [f32; 4],
    pub kb_a: [f32; 4],
    pub kb_b: [f32; 4],
    pub strike_color: [f32; 4],
    pub bg: [f32; 4],
    pub bg_color: [f32; 4],
    pub bg_image: [f32; 4],
    pub stops: [[f32; 4]; MAX_STOPS],
    pub palette: [[f32; 4]; MAX_PALETTE],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct KeyGpu {
    pub x0: f32,
    pub x1: f32,
    pub black: f32,
    pub visible: f32,
    pub note: i32,
    pub press: f32,
    pub age: f32,
    pub velocity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct EmitterGpu {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub d: [f32; 4],
    pub e: [f32; 4],
    pub f: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct ParticlesGpu {
    pub em: [EmitterGpu; 4],
    pub info: [u32; 4],
    pub offs: [u32; 4],
    pub counts: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct PostGpu {
    pub bloom: [f32; 4],
    pub bloom_tint: [f32; 4],
    pub tone: [f32; 4],
    pub vignette: [f32; 4],
    pub misc: [f32; 4],
    pub reflect: [f32; 4],
    pub out_size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct BloomPass {
    pub texel: [f32; 2],
    pub weight: f32,
    pub prefilter: f32,
    pub threshold: f32,
    pub knee: f32,
    pub _pad: [f32; 2],
}

/// Pixel geometry of the frame, derived from the Design and target size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub width: f32,
    pub height: f32,
    /// Pixels per 1080p pixel: Design sizes are authored at 1080p.
    pub px: f32,
    /// Bottom edge of the keyboard; above zero when a reflection sits below.
    pub kb_y0: f32,
    pub kb_h: f32,
    pub pps: f32,
}

impl Frame {
    pub fn new(d: &Design, width: f32, height: f32) -> Self {
        let kb_h = d.keyboard.height * height;
        let kb_y0 = if d.background.reflection.enabled { (kb_h * 0.6).round() } else { 0.0 };
        let pps = (height - kb_y0 - kb_h).max(1.0) / d.layout.lookahead;
        Self { width, height, px: height / 1080.0, kb_y0, kb_h, pps }
    }
}

fn lin(c: pv_design::Color) -> [f32; 4] {
    c.linear()
}

fn lin3(c: pv_design::Color, w: f32) -> [f32; 4] {
    let [r, g, b, _] = c.linear();
    [r, g, b, w]
}

/// Everything in [`Globals`] that depends only on the Design and target,
/// not on the moment being drawn.
pub fn globals(
    d: &Design,
    f: &Frame,
    keys: &KeyLayout,
    split: u8,
    image_aspect: Option<f32>,
) -> Globals {
    let n = &d.notes;
    let kb = &d.keyboard;
    let px = f.px;
    let mut stops = [[0.0; 4]; MAX_STOPS];
    let mut sorted = d.background.stops.clone();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
    for (s, st) in stops.iter_mut().zip(&sorted) {
        *s = lin3(st.color, st.at);
    }
    let mut palette = [[0.0; 4]; MAX_PALETTE];
    for (p, c) in palette.iter_mut().zip(&n.color.palette) {
        *p = lin(*c);
    }
    let dir = match d.layout.direction {
        pv_design::Direction::Down => 1.0,
        pv_design::Direction::Up => -1.0,
    };
    Globals {
        size: [f.width, f.height, 1.0 / f.width, 1.0 / f.height],
        view: [0.0, px, 0.0, 1.0],
        layout: [f.kb_y0, f.kb_h, f.pps, dir],
        camera: [0.0, 0.0, d.camera.zoom, n.color.split.unwrap_or(split) as f32],
        keys: [
            keys.lo as u32,
            keys.hi as u32,
            match n.color.source {
                ColorSource::Track => 0,
                ColorSource::Channel => 1,
                ColorSource::Hand => 2,
                ColorSource::PitchClass => 3,
                ColorSource::Fixed => 4,
                ColorSource::Velocity => 5,
            },
            n.color.palette.len().clamp(1, MAX_PALETTE) as u32,
        ],
        note_a: [n.corner_radius * px, n.border_width * px, n.opacity, n.intensity],
        note_b: [
            match n.gradient {
                GradientDir::None => 0.0,
                GradientDir::Along => 1.0,
                GradientDir::Across => 2.0,
            },
            n.gradient_falloff,
            n.margin * px,
            n.active_boost,
        ],
        note_c: [
            n.color.velocity_brightness,
            n.color.black_key_shade,
            if n.sustain_tail.enabled { n.sustain_tail.opacity } else { 0.0 },
            0.0,
        ],
        border_color: lin(n.border_color),
        white_key: lin(kb.white_key_color),
        black_key: lin(kb.black_key_color),
        separator: lin3(kb.separator_color, kb.separator_width * px),
        kb_a: [
            kb.black_width_ratio,
            kb.black_length_ratio,
            kb.pressed.depress_pixels * px,
            if kb.pressed.tint_from_note { kb.pressed.tint_amount } else { 0.0 },
        ],
        kb_b: [
            kb.pressed.glow_intensity,
            kb.pressed.glow_radius * px,
            kb.strike_line.width * px,
            if kb.strike_line.enabled { kb.strike_line.intensity } else { 0.0 },
        ],
        strike_color: lin(kb.strike_line.color),
        bg: [
            match d.background.kind {
                BackgroundKind::Solid => 0.0,
                BackgroundKind::Gradient => 1.0,
                BackgroundKind::Image => 2.0,
            },
            sorted.len().min(MAX_STOPS) as f32,
            d.background.angle.to_radians(),
            if d.background.radial { 1.0 } else { 0.0 },
        ],
        bg_color: if d.background.kind == BackgroundKind::Image {
            lin(d.background.tint)
        } else {
            lin(d.background.color)
        },
        bg_image: [
            match d.background.fit {
                ImageFit::Fill => 0.0,
                ImageFit::Fit => 1.0,
                ImageFit::Stretch => 2.0,
            },
            image_aspect.unwrap_or(1.0),
            if image_aspect.is_some() { 1.0 } else { 0.0 },
            0.0,
        ],
        stops,
        palette,
    }
}

fn curve(c: Curve) -> f32 {
    match c {
        Curve::Constant => 0.0,
        Curve::Shrink => 1.0,
        Curve::Grow => 2.0,
        Curve::Pulse => 3.0,
        Curve::FadeOut => 4.0,
    }
}

/// Emitter parameters plus the per-note instance layout, and the longest
/// particle lifetime per event (hit, hold, release) for the CPU's
/// active-note query.
pub fn particles(d: &Design) -> (ParticlesGpu, [f32; 3]) {
    let mut out = ParticlesGpu::default();
    let mut max_life = [0.0f32; 3];
    let mut n = 0usize;
    let mut offset = 0u32;
    for e in d.particles.iter().filter(|e| e.enabled).take(4) {
        let (event, instances) = match e.event {
            EmitEvent::NoteHit => (0, burst_instances(e)),
            EmitEvent::NoteHold => (1, e.max_alive_per_note()),
            EmitEvent::NoteRelease => (2, burst_instances(e)),
        };
        if instances == 0 {
            continue;
        }
        max_life[event] = max_life[event].max(e.lifetime);
        let [r, g, b, _] = e.color.linear();
        out.em[n] = EmitterGpu {
            a: [event as f32, e.count as f32, e.rate, e.lifetime],
            b: [e.lifetime_variance, e.speed, e.speed_variance, e.direction_degrees.to_radians()],
            c: [e.spread_degrees.to_radians(), e.gravity[0], e.gravity[1], e.drag],
            d: [e.size, e.size_variance, curve(e.size_curve), curve(e.opacity_curve)],
            e: [
                e.turbulence,
                match e.color_source {
                    ParticleColor::Note => 0.0,
                    ParticleColor::Palette => 1.0,
                    ParticleColor::Fixed => 2.0,
                },
                e.white_hot,
                e.intensity,
            ],
            f: [r, g, b, e.velocity_response],
        };
        out.offs[n] = offset;
        out.counts[n] = instances;
        offset += instances;
        n += 1;
    }
    out.info = [n as u32, offset, 0, 0];
    (out, max_life)
}

/// Loud notes throw more particles: up to 1.45x the count at full velocity
/// and full velocity response, which must fit in the instance block.
fn burst_instances(e: &pv_design::Emitter) -> u32 {
    (e.count as f32 * (1.0 + 0.45 * e.velocity_response.clamp(0.0, 1.0))).ceil() as u32
}

pub fn post(
    d: &Design,
    f: &Frame,
    out_w: u32,
    out_h: u32,
    supersample: u32,
    alpha: bool,
    encode_srgb: bool,
) -> PostGpu {
    let p = &d.post;
    let r = &d.background.reflection;
    PostGpu {
        bloom: [if p.bloom.enabled { p.bloom.intensity } else { 0.0 }, 0.0, 0.0, 0.0],
        bloom_tint: lin(p.bloom.tint),
        tone: [
            match p.tonemap.operator {
                TonemapOp::Agx => 0.0,
                TonemapOp::Aces => 1.0,
                TonemapOp::Reinhard => 2.0,
                TonemapOp::None => 3.0,
            },
            2f32.powf(p.tonemap.exposure),
            p.tonemap.saturation,
            if alpha { 1.0 } else { 0.0 },
        ],
        vignette: [
            if p.vignette.enabled { p.vignette.amount } else { 0.0 },
            p.vignette.smoothness,
            if p.grain.enabled { p.grain.amount } else { 0.0 },
            0.0,
        ],
        misc: [
            if p.chromatic_aberration.enabled { p.chromatic_aberration.amount } else { 0.0 },
            if encode_srgb { 1.0 } else { 0.0 },
            supersample as f32,
            1.0,
        ],
        reflect: [
            if r.enabled && !alpha { r.opacity } else { 0.0 },
            r.fade,
            r.blur * out_h as f32 / 1080.0,
            f.kb_y0 / f.height,
        ],
        out_size: [out_w as f32, out_h as f32, 1.0 / out_w as f32, 1.0 / out_h as f32],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_multiples_of_16() {
        // WGSL uniform structs are 16-byte aligned; a mismatch here means
        // the shader reads garbage.
        for (name, size) in [
            ("Globals", std::mem::size_of::<Globals>()),
            ("ParticlesGpu", std::mem::size_of::<ParticlesGpu>()),
            ("PostGpu", std::mem::size_of::<PostGpu>()),
            ("BloomPass", std::mem::size_of::<BloomPass>()),
            ("KeyGpu", std::mem::size_of::<KeyGpu>()),
        ] {
            assert_eq!(size % 16, 0, "{name} is {size} bytes");
        }
        assert_eq!(std::mem::size_of::<Globals>(), 18 * 16 + (MAX_STOPS + MAX_PALETTE) * 16);
    }

    #[test]
    fn instance_blocks_cover_velocity_boost() {
        let d = Design::default();
        let (p, life) = particles(&d);
        assert_eq!(p.info[0], 2);
        assert_eq!(p.offs[1], p.counts[0]);
        assert_eq!(p.info[1], p.counts[0] + p.counts[1]);
        assert!(p.counts[0] as f32 >= d.particles[0].count as f32 * 1.3);
        assert!(life[0] > 0.0 && life[1] > 0.0 && life[2] == 0.0);
    }

    #[test]
    fn disabled_emitters_are_skipped() {
        let mut d = Design::default();
        d.particles[0].enabled = false;
        let (p, _) = particles(&d);
        assert_eq!(p.info[0], 1);
        assert_eq!(p.em[0].a[0], 1.0, "the hold emitter moved to slot 0");
    }
}
