//! Export guarantees: preview/export parity, reproducibility, clean
//! failures. GPU tests skip without an adapter unless PV_REQUIRE_GPU is set;
//! ffmpeg tests skip without ffmpeg.

use pv_core::{Clock, FrameClock, Rational};
use pv_design::Design;
use pv_export::{ExportError, Job, Preset, Settings, export, frame_count, validate};
use pv_render::{FrameParams, Gpu, Offscreen, Renderer, render_image};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var_os("PV_REQUIRE_GPU").is_none() => {
            eprintln!("skipping: {e}");
            None
        }
        Err(e) => panic!("{e}"),
    })
    .as_ref()
}

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_file(&d);
    d
}

fn job(settings: Settings) -> Job {
    let score = Arc::new(pv_midi::parse(&pv_midi::demo::prelude()).unwrap());
    Job {
        track_visible: vec![true; score.tracks.len()],
        track_muted: vec![false; score.tracks.len()],
        score,
        design: Design::default(),
        bundle_dir: None,
        font: pv_audio::builtin_font(),
        sequence: Default::default(),
        volume: 1.0,
        settings,
    }
}

fn small(preset: Preset, output: PathBuf) -> Settings {
    Settings {
        width: 160,
        height: 90,
        fps: Rational::new(30, 1),
        supersample: 1,
        start: 20.0,
        end: Some(21.0),
        preset,
        output,
        audio: false,
        ..Default::default()
    }
}

#[test]
fn frame_counts_round_up() {
    assert_eq!(frame_count(1.0, Rational::new(30, 1)), 30);
    assert_eq!(frame_count(1.01, Rational::new(30, 1)), 31);
    assert_eq!(frame_count(0.0, Rational::new(30, 1)), 0);
    assert_eq!(frame_count(1001.0 / 30000.0 * 100.0, Rational::new(30000, 1001)), 100);
}

#[test]
fn bad_settings_fail_before_rendering() {
    let bad = |f: fn(&mut Settings)| {
        let mut s = small(Preset::H264 { crf: 18 }, "x.mp4".into());
        f(&mut s);
        validate(&s).unwrap_err()
    };
    assert!(bad(|s| s.width = 161).to_string().contains("even"));
    assert!(bad(|s| s.alpha = true).to_string().contains("alpha"));
    assert!(bad(|s| s.height = 4).to_string().contains("out of range"));
    assert!(
        bad(|s| s.ffmpeg = Some("/nonexistent/ffmpeg".into())).to_string().contains("no ffmpeg at")
    );
}

#[test]
fn exported_frames_match_preview_exactly() {
    // The core promise: frame n of an export is the preview at the same
    // moment, byte for byte.
    let Some(gpu) = gpu() else { return };
    let dir = scratch("parity");
    let s = small(Preset::PngSequence, dir.clone());
    let j = job(s.clone());
    let summary = export(gpu, &j, |_| true).unwrap();
    assert_eq!(summary.frames, 30);

    let mut r = Renderer::new(gpu, pv_render::CAPTURE_FORMAT, s.width, s.height, s.supersample);
    r.set_score(gpu, j.score.clone());
    r.set_design(gpu, &j.design, None);
    let target = Offscreen::new(gpu, s.width, s.height);
    for n in [0u64, 7, 29] {
        let t = FrameClock::at(s.fps, s.start, n).now();
        let preview =
            render_image(gpu, &mut r, &target, FrameParams { time: t, frame: 0, alpha: false });
        let bytes = std::fs::read(dir.join(format!("frame_{n:06}.png"))).unwrap();
        let exported = pv_render::decode_image(&bytes).unwrap().2;
        assert!(preview == exported, "frame {n} differs from the preview at t={t}");
    }
}

#[test]
fn png_sequence_with_audio_writes_a_wav() {
    let Some(gpu) = gpu() else { return };
    let dir = scratch("with-audio");
    let mut s = small(Preset::PngSequence, dir.clone());
    s.audio = true;
    export(gpu, &job(s), |_| true).unwrap();
    let wav = std::fs::read(dir.join("audio.wav")).unwrap();
    // One second of 48 kHz stereo f32, plus the header.
    assert_eq!(wav.len(), 44 + 48_000 * 2 * 4);
}

#[test]
fn alpha_png_export_is_transparent() {
    let Some(gpu) = gpu() else { return };
    let dir = scratch("alpha");
    let mut s = small(Preset::PngSequence, dir.clone());
    s.alpha = true;
    export(gpu, &job(s), |_| true).unwrap();
    let img =
        pv_render::decode_image(&std::fs::read(dir.join("frame_000000.png")).unwrap()).unwrap().2;
    assert_eq!(img[3], 0, "top-left should be transparent");
}

#[test]
fn cancelling_stops_early() {
    let Some(gpu) = gpu() else { return };
    let dir = scratch("cancel");
    let mut calls = 0;
    let r = export(gpu, &job(small(Preset::PngSequence, dir.clone())), |_| {
        calls += 1;
        calls < 5
    });
    assert!(matches!(r, Err(ExportError::Cancelled)));
    let written = std::fs::read_dir(&dir).unwrap().count();
    assert!(written < 30, "kept rendering after cancel: {written} frames");
}

fn have_ffmpeg() -> bool {
    pv_export::ffmpeg::find(None).is_some()
}

#[test]
fn mp4_export_has_video_and_audio_of_the_right_length() {
    let Some(gpu) = gpu() else { return };
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let out = scratch("clip.mp4");
    let mut s = small(Preset::H264 { crf: 23 }, out.clone());
    s.audio = true;
    export(gpu, &job(s), |_| true).unwrap();
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type,duration", "-of", "csv=p=0"])
        .arg(&out)
        .output();
    let Ok(probe) = probe else { return };
    let text = String::from_utf8_lossy(&probe.stdout);
    assert!(text.contains("video,1.0") && text.contains("audio,1.0"), "{text}");
}

#[test]
fn cancelled_video_leaves_no_partial_file() {
    let Some(gpu) = gpu() else { return };
    if !have_ffmpeg() {
        return;
    }
    let out = scratch("partial.mp4");
    let mut n = 0;
    let r = export(gpu, &job(small(Preset::H264 { crf: 23 }, out.clone())), |_| {
        n += 1;
        n < 10
    });
    assert!(matches!(r, Err(ExportError::Cancelled)));
    assert!(!out.exists(), "partial file left behind");
}
