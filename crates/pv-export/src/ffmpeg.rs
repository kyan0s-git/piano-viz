//! Encoding by piping raw frames to an `ffmpeg` process.
//!
//! Linking FFmpeg would mean LGPL at minimum and GPL with x264, forcing
//! this project's license. Running a separate executable carries no such
//! obligation, and every codec FFmpeg has works with no per-format code.

use crate::ExportError;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// H.264 in MP4, for YouTube and everything else.
    H264 { crf: u8 },
    /// H.265 in MP4: smaller at the same quality.
    H265 { crf: u8 },
    /// ProRes 4444 with alpha, for Premiere / After Effects / Resolve.
    ProRes4444,
    /// VP9 with alpha in WebM, for web compositing.
    Vp9Alpha,
    /// FFV1 in MKV: lossless, archival.
    Ffv1,
    /// Numbered PNGs plus a WAV. Needs no ffmpeg.
    PngSequence,
}

impl Preset {
    pub fn has_alpha(self) -> bool {
        matches!(self, Self::ProRes4444 | Self::Vp9Alpha | Self::PngSequence)
    }

    pub fn needs_ffmpeg(self) -> bool {
        self != Self::PngSequence
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::H264 { .. } | Self::H265 { .. } => "mp4",
            Self::ProRes4444 => "mov",
            Self::Vp9Alpha => "webm",
            Self::Ffv1 => "mkv",
            Self::PngSequence => "",
        }
    }

    /// The ffmpeg encoder this preset requires, to check up front.
    pub fn encoder(self) -> Option<&'static str> {
        Some(match self {
            Self::H264 { .. } => "libx264",
            Self::H265 { .. } => "libx265",
            Self::ProRes4444 => "prores_ks",
            Self::Vp9Alpha => "libvpx-vp9",
            Self::Ffv1 => "ffv1",
            Self::PngSequence => return None,
        })
    }

    /// Video and audio codec arguments.
    fn args(self) -> Vec<String> {
        // RGB->YUV must use BT.709 for HD, and say so; ffmpeg's default
        // BT.601 matrix visibly shifts reds and greens.
        let bt709 = |fmt: &str| {
            vec![
                "-vf".into(),
                format!("scale=out_color_matrix=bt709:out_range=tv,format={fmt}"),
                "-colorspace".into(),
                "bt709".into(),
                "-color_primaries".into(),
                "bt709".into(),
                "-color_trc".into(),
                "bt709".into(),
                "-color_range".into(),
                "tv".into(),
            ]
        };
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        match self {
            Self::H264 { crf } => [
                s(&["-c:v", "libx264", "-preset", "medium", "-crf", &crf.to_string()]),
                bt709("yuv420p"),
                s(&["-movflags", "+faststart", "-c:a", "aac", "-b:a", "320k"]),
            ]
            .concat(),
            Self::H265 { crf } => [
                s(&[
                    "-c:v",
                    "libx265",
                    "-preset",
                    "medium",
                    "-crf",
                    &crf.to_string(),
                    "-tag:v",
                    "hvc1",
                ]),
                bt709("yuv420p"),
                s(&["-movflags", "+faststart", "-c:a", "aac", "-b:a", "320k"]),
            ]
            .concat(),
            Self::ProRes4444 => s(&[
                "-c:v",
                "prores_ks",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "yuva444p10le",
                "-alpha_bits",
                "16",
                "-vendor",
                "apl0",
                "-c:a",
                "pcm_s16le",
            ]),
            Self::Vp9Alpha => s(&[
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuva420p",
                "-b:v",
                "0",
                "-crf",
                "28",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "192k",
            ]),
            Self::Ffv1 => s(&["-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgr0", "-c:a", "flac"]),
            Self::PngSequence => Vec::new(),
        }
    }
}

/// Where ffmpeg is: an explicit path (used as given, never second-guessed),
/// else next to our binary, else on PATH.
pub fn find(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let candidates = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(exe)))
        .into_iter()
        .chain(
            std::env::var_os("PATH")
                .into_iter()
                .flat_map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect::<Vec<_>>()),
        );
    candidates.into_iter().find(|c| c.is_file())
}

/// Confirm ffmpeg runs and has the encoder, before any frame is rendered —
/// never a failure discovered at the end of a long render.
pub fn check(ffmpeg: &Path, preset: Preset) -> Result<(), ExportError> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map_err(|e| ExportError::Ffmpeg(format!("could not run {}: {e}", ffmpeg.display())))?;
    let list = String::from_utf8_lossy(&out.stdout);
    if let Some(enc) = preset.encoder()
        && !list.split_whitespace().any(|w| w == enc)
    {
        return Err(ExportError::Ffmpeg(format!(
            "this ffmpeg has no `{enc}` encoder; install a full build or pick another format"
        )));
    }
    Ok(())
}

pub struct Spec<'a> {
    pub ffmpeg: &'a Path,
    pub width: u32,
    pub height: u32,
    pub fps: pv_core::Rational,
    pub audio: Option<&'a Path>,
    pub preset: Preset,
    pub output: &'a Path,
}

pub fn spawn(spec: &Spec) -> Result<std::process::Child, ExportError> {
    let mut cmd = Command::new(spec.ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", spec.width, spec.height)])
        .args(["-r", &format!("{}/{}", spec.fps.num, spec.fps.den)])
        .args(["-i", "-"]);
    if let Some(a) = spec.audio {
        cmd.arg("-i").arg(a).args(["-map", "0:v", "-map", "1:a"]);
    }
    cmd.args(spec.preset.args()).arg(spec.output);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.spawn()
        .map_err(|e| ExportError::Ffmpeg(format!("could not start {}: {e}", spec.ffmpeg.display())))
}
