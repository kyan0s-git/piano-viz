//! Rendering tests: the guarantees docs/08-export.md makes, plus golden
//! images. They need a GPU adapter; a software one (lavapipe, WARP) is fine.
//! Without any adapter they skip, unless PV_REQUIRE_GPU is set (CI sets it).

use pv_design::Design;
use pv_render::{FrameParams, Gpu, Offscreen, Renderer, render_image};
use std::sync::{Arc, OnceLock};

const W: u32 = 320;
const H: u32 = 180;

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var_os("PV_REQUIRE_GPU").is_none() => {
            eprintln!("skipping GPU tests: {e}");
            None
        }
        Err(e) => panic!("PV_REQUIRE_GPU is set but no adapter: {e}"),
    })
    .as_ref()
}

fn demo() -> Arc<pv_core::Score> {
    static SCORE: OnceLock<Arc<pv_core::Score>> = OnceLock::new();
    SCORE.get_or_init(|| Arc::new(pv_midi::parse(&pv_midi::demo::prelude()).unwrap())).clone()
}

struct Rig {
    gpu: &'static Gpu,
    r: Renderer,
    target: Offscreen,
}

impl Rig {
    fn new(design: &Design) -> Option<Self> {
        let gpu = gpu()?;
        let mut r = Renderer::new(gpu, pv_render::CAPTURE_FORMAT, W, H, 2);
        r.set_score(gpu, demo());
        r.set_design(gpu, design, None);
        Some(Self { gpu, r, target: Offscreen::new(gpu, W, H) })
    }

    fn at(&mut self, time: f64) -> Vec<u8> {
        self.frame(FrameParams { time, frame: 0, alpha: false })
    }

    fn frame(&mut self, p: FrameParams) -> Vec<u8> {
        render_image(self.gpu, &mut self.r, &self.target, p)
    }
}

#[test]
fn same_frame_twice_is_bit_identical() {
    let Some(mut rig) = Rig::new(&Design::default()) else { return };
    assert!(rig.at(20.0) == rig.at(20.0));
}

#[test]
fn seeking_lands_on_the_same_frame_as_playing() {
    // The claim stateless particles exist to deliver.
    let Some(mut played) = Rig::new(&Design::default()) else { return };
    for i in 0..60 {
        played.at(i as f64 * 0.5);
    }
    let after_playing = played.at(30.25);
    let Some(mut fresh) = Rig::new(&Design::default()) else { return };
    assert!(fresh.at(30.25) == after_playing);
}

#[test]
fn frame_number_does_not_change_the_picture() {
    // Same moment at 30 fps and 60 fps: different frame numbers, same image.
    let Some(mut rig) = Rig::new(&Design::default()) else { return };
    let a = rig.frame(FrameParams { time: 14.0, frame: 420, alpha: false });
    let b = rig.frame(FrameParams { time: 14.0, frame: 840, alpha: false });
    assert!(a == b);
}

#[test]
fn alpha_export_is_transparent_where_nothing_is_drawn() {
    let Some(mut rig) = Rig::new(&Design::default()) else { return };
    let img = rig.frame(FrameParams { time: 12.0, frame: 0, alpha: true });
    let alpha = |x: u32, y: u32| img[((y * W + x) * 4 + 3) as usize];
    assert_eq!(alpha(2, 2), 0, "empty corner should be transparent");
    assert_eq!(alpha(W / 2, H - 4), 255, "keyboard should be opaque");
}

#[test]
fn hidden_tracks_draw_nothing() {
    let Some(mut rig) = Rig::new(&Design::default()) else { return };
    let with_notes = rig.at(12.0);
    rig.r.set_track_visibility(rig.gpu, &[false; 3]);
    let hidden = rig.at(12.0);
    let r = &mut Rig::new(&Design::default()).unwrap();
    r.r.set_score(r.gpu, Arc::new(pv_core::Score::default()));
    let empty = r.at(12.0);
    assert!(hidden != with_notes);
    // Upper half (the note field) should match an empty score exactly.
    let half = (W * H / 2 * 4) as usize;
    assert!(hidden[..half] == empty[..half]);
}

#[test]
fn every_builtin_design_renders() {
    for (id, _) in pv_design::BUILTIN {
        let Some(mut rig) = Rig::new(&pv_design::builtin(id)) else { return };
        let img = rig.at(25.5);
        let lit = img
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 30)
            .count();
        assert!(lit > (W * H / 20) as usize, "{id} rendered almost nothing");
    }
}

#[test]
fn empty_score_renders_keyboard_only() {
    let Some(gpu) = gpu() else { return };
    let mut r = Renderer::new(gpu, pv_render::CAPTURE_FORMAT, W, H, 1);
    r.set_design(gpu, &Design::default(), None);
    let target = Offscreen::new(gpu, W, H);
    render_image(gpu, &mut r, &target, FrameParams { time: 0.0, frame: 0, alpha: false });
    assert_eq!(r.stats().visible_notes, 0);
}

#[test]
fn odd_sizes_read_back_without_skew() {
    // 333 px rows aren't 256-byte aligned: exercises padding removal.
    let Some(gpu) = gpu() else { return };
    let (w, h) = (333, 101);
    let mut r = Renderer::new(gpu, pv_render::CAPTURE_FORMAT, w, h, 1);
    r.set_score(gpu, demo());
    r.set_design(gpu, &pv_design::builtin("print"), None);
    let img = render_image(
        gpu,
        &mut r,
        &Offscreen::new(gpu, w, h),
        FrameParams { time: 5.0, frame: 0, alpha: false },
    );
    assert_eq!(img.len(), (w * h * 4) as usize);
    // Print's background is pure white; the top-left and top-right corners
    // must both be white if rows line up.
    let px = |x: u32, y: u32| &img[((y * w + x) * 4) as usize..][..3];
    assert_eq!(px(0, 0), [255, 255, 255]);
    assert_eq!(px(w - 1, 0), [255, 255, 255]);
}

/// Compare against committed references. Set PV_BLESS=1 to regenerate them,
/// and review the new images in the diff like any other change.
#[test]
fn golden_images() {
    let cases = [("ember-classic", 12.3), ("aurora", 25.5), ("neon", 40.0), ("minimal", 25.5)];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let bless = std::env::var_os("PV_BLESS").is_some();
    let mut failures = Vec::new();
    for (id, t) in cases {
        let Some(mut rig) = Rig::new(&pv_design::builtin(id)) else { return };
        let img = rig.at(t);
        let path = dir.join(format!("{id}.png"));
        if bless {
            write_png(&path, &img);
            continue;
        }
        let want = read_png(&path);
        // Mean error allows for adapters differing in the last bit of
        // floating point; the max catches a feature going missing.
        let diffs: Vec<u32> = img.iter().zip(&want).map(|(a, b)| a.abs_diff(*b) as u32).collect();
        let mean = diffs.iter().sum::<u32>() as f64 / diffs.len() as f64;
        let big = diffs.iter().filter(|&&d| d > 40).count() as f64 / diffs.len() as f64;
        if mean > 1.5 || big > 0.002 {
            let out =
                std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{id}-actual.png"));
            write_png(&out, &img);
            failures.push(format!(
                "{id}: mean diff {mean:.3}, {:.3}% pixels off; actual at {}",
                big * 100.0,
                out.display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn write_png(path: &std::path::Path, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn read_png(path: &std::path::Path) -> Vec<u8> {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("{}: {e} (run with PV_BLESS=1 to create)", path.display()));
    pv_render::decode_image(&bytes).unwrap().2
}

#[test]
fn live_updates_draw_rising_notes_above_the_keys() {
    // Live mode: a changing score, notes rising from the keys. A held note
    // ends "now", so under the falling direction it would sit entirely below
    // the strike line and vanish — the bug this guards against.
    let mut d = Design::default();
    d.layout.direction = pv_design::Direction::Up;
    d.particles.clear();
    // Only the note may change the picture above the keys: no key glow.
    d.keyboard.pressed.glow_intensity = 0.0;
    d.keyboard.pressed.tint_from_note = false;
    let Some(mut rig) = Rig::new(&d) else { return };
    let empty = Arc::new(pv_core::Score::default());
    rig.r.update_score(rig.gpu, empty);
    let before = rig.at(2.0);
    let held = pv_core::Note {
        start: 1.0,
        duration: 1.0,
        sustain: 0.0,
        pitch: 60,
        velocity: 100,
        track: 0,
        flags: 0,
    };
    let score = pv_core::Score { notes: pv_core::NoteTable::new(vec![held]), ..Default::default() };
    rig.r.update_score(rig.gpu, Arc::new(score));
    let after = rig.at(2.0);
    // Compare the band just above the keyboard (rows 40%-75% from the top).
    let band = |img: &[u8]| -> u64 {
        let (a, b) = ((H * 40 / 100) as usize, (H * 75 / 100) as usize);
        img[a * W as usize * 4..b * W as usize * 4].iter().map(|&v| v as u64).sum()
    };
    assert!(band(&after) > band(&before) + 20_000, "no rising note drawn above the keys");
}
