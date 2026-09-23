//! Video export. See docs/08-export.md.
//!
//! The same renderer as preview, driven by a [`FrameClock`] instead of the
//! audio clock and drawing offscreen instead of to a window. Nothing else
//! differs, which is why what you preview is what you get.

pub mod ffmpeg;

pub use ffmpeg::Preset;

use pv_audio::SoundFont;
use pv_audio::offline::{self, OfflineOptions};
use pv_audio::sequencer::{SequenceOptions, TrackMask};
use pv_core::{Clock, FrameClock, Rational, Score};
use pv_design::Design;
use pv_render::{FrameParams, Gpu, Offscreen, Renderer, padded_row, unpad, wgpu};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Ffmpeg(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Audio(String),
    #[error("export cancelled")]
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub supersample: u32,
    /// Seconds into the song where the video starts.
    pub start: f64,
    /// Where it ends; `None` means the last note plus `tail`.
    pub end: Option<f64>,
    /// Extra seconds after the end so particles and glow finish fading
    /// instead of cutting off. `None` picks the longest particle lifetime
    /// plus a second.
    pub tail: Option<f64>,
    pub preset: Preset,
    /// Transparent background. Needs a preset that carries alpha.
    pub alpha: bool,
    /// A file for video presets; a directory for PNG sequences.
    pub output: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    pub audio: bool,
    pub sample_rate: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: Rational::new(60, 1),
            supersample: 2,
            start: 0.0,
            end: None,
            tail: None,
            preset: Preset::H264 { crf: 18 },
            alpha: false,
            output: PathBuf::from("out.mp4"),
            ffmpeg: None,
            audio: true,
            sample_rate: 48_000,
        }
    }
}

/// Everything an export needs.
pub struct Job {
    pub score: Arc<Score>,
    pub design: Design,
    pub bundle_dir: Option<PathBuf>,
    pub font: Arc<SoundFont>,
    pub track_visible: Vec<bool>,
    pub track_muted: Vec<bool>,
    pub sequence: SequenceOptions,
    pub volume: f32,
    pub settings: Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Audio,
    Video,
    Finishing,
}

pub struct Progress<'a> {
    pub stage: Stage,
    pub frame: u64,
    pub total: u64,
    pub elapsed: Duration,
    pub eta: Option<Duration>,
    /// The frame just rendered (RGBA8, output size), offered a few times a
    /// second for a live thumbnail.
    pub preview: Option<&'a [u8]>,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub frames: u64,
    pub seconds: f64,
    pub elapsed: Duration,
    pub output: PathBuf,
}

/// Default tail: long enough for the longest particle to finish.
pub fn default_tail(design: &Design) -> f64 {
    design.particles.iter().filter(|e| e.enabled).map(|e| e.lifetime as f64).fold(0.0, f64::max)
        + 1.0
}

/// The time range an export covers.
pub fn range(score: &Score, design: &Design, s: &Settings) -> (f64, f64) {
    let end =
        s.end.unwrap_or_else(|| score.duration + s.tail.unwrap_or_else(|| default_tail(design)));
    (s.start.max(0.0), end.max(s.start.max(0.0)))
}

/// Frames needed to cover `seconds` at `fps`, rounding up.
pub fn frame_count(seconds: f64, fps: Rational) -> u64 {
    (seconds * fps.num as f64 / fps.den as f64 - 1e-9).ceil().max(0.0) as u64
}

/// Check settings and tools before any work, so a bad setting or a missing
/// encoder is reported immediately, not after a long render.
pub fn validate(s: &Settings) -> Result<Option<PathBuf>, ExportError> {
    if s.width < 16 || s.height < 16 || s.width > 16384 || s.height > 16384 {
        return Err(ExportError::Invalid(format!(
            "resolution {}x{} is out of range",
            s.width, s.height
        )));
    }
    if s.preset != Preset::PngSequence && (s.width % 2 == 1 || s.height % 2 == 1) {
        return Err(ExportError::Invalid("video encoders need an even width and height".into()));
    }
    if s.alpha && !s.preset.has_alpha() {
        return Err(ExportError::Invalid(
            "this format has no alpha channel; use ProRes 4444, WebM, or a PNG sequence".into(),
        ));
    }
    if !s.preset.needs_ffmpeg() {
        return Ok(None);
    }
    let bin = ffmpeg::find(s.ffmpeg.as_deref()).ok_or_else(|| match &s.ffmpeg {
        Some(p) => ExportError::Ffmpeg(format!("no ffmpeg at {}", p.display())),
        None => ExportError::Ffmpeg(
            "ffmpeg was not found. Install it (https://ffmpeg.org/download.html), set its path, \
             or export a PNG sequence, which needs no ffmpeg"
                .into(),
        ),
    })?;
    ffmpeg::check(&bin, s.preset)?;
    Ok(Some(bin))
}

/// Render the job. `progress` returns `false` to cancel.
pub fn export(
    gpu: &Gpu,
    job: &Job,
    mut progress: impl FnMut(&Progress) -> bool,
) -> Result<Summary, ExportError> {
    let started = Instant::now();
    let s = &job.settings;
    let ffmpeg_bin = validate(s)?;
    let (t0, t1) = range(&job.score, &job.design, s);
    let total = frame_count(t1 - t0, s.fps);
    if total == 0 {
        return Err(ExportError::Invalid("nothing to render: the range is empty".into()));
    }

    // ── Audio first: fast, and ffmpeg wants it as an input file ──
    let audio_path = if s.audio {
        let opts = OfflineOptions {
            sample_rate: s.sample_rate,
            sequence: job.sequence,
            muted: TrackMask::from_muted(&job.track_muted),
            volume: job.volume,
        };
        let mut cancelled = false;
        let pcm = offline::render(&job.score, &job.font, &opts, t0, t1, |f| {
            let p = Progress {
                stage: Stage::Audio,
                frame: (f as f64 * total as f64) as u64,
                total,
                elapsed: started.elapsed(),
                eta: None,
                preview: None,
            };
            cancelled |= !progress(&p);
        })
        .map_err(|e| ExportError::Audio(e.to_string()))?;
        if cancelled {
            return Err(ExportError::Cancelled);
        }
        let path = if s.preset == Preset::PngSequence {
            std::fs::create_dir_all(&s.output).map_err(|e| io(&s.output, e))?;
            s.output.join("audio.wav")
        } else {
            std::env::temp_dir().join(format!(
                "piano-viz-{}-{}.wav",
                std::process::id(),
                started.elapsed().as_nanos()
            ))
        };
        let f = std::fs::File::create(&path).map_err(|e| io(&path, e))?;
        pv_audio::wav::write_f32(std::io::BufWriter::new(f), &pcm, 2, s.sample_rate)
            .map_err(|e| io(&path, e))?;
        Some(path)
    } else {
        None
    };
    // The temporary WAV is removed however this function exits.
    let _cleanup = TempFile(audio_path.clone().filter(|_| s.preset != Preset::PngSequence));

    // ── Sink: ffmpeg or PNG files, fed from a writer thread ──
    let sink = match &ffmpeg_bin {
        Some(bin) => {
            if let Some(dir) = s.output.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
            }
            Sink::Ffmpeg(ffmpeg::spawn(&ffmpeg::Spec {
                ffmpeg: bin,
                width: s.width,
                height: s.height,
                fps: s.fps,
                audio: audio_path.as_deref(),
                preset: s.preset,
                output: &s.output,
            })?)
        }
        None => {
            std::fs::create_dir_all(&s.output).map_err(|e| io(&s.output, e))?;
            Sink::Png(s.output.clone())
        }
    };
    let mut writer = Writer::start(sink, s.width, s.height);

    // ── Frames ──
    let mut renderer =
        Renderer::new(gpu, pv_render::CAPTURE_FORMAT, s.width, s.height, s.supersample);
    renderer.set_score(gpu, job.score.clone());
    let warnings = renderer.set_design(gpu, &job.design, job.bundle_dir.as_deref());
    for w in warnings {
        eprintln!("warning: {w}");
    }
    renderer.set_track_visibility(gpu, &job.track_visible);
    let target = Offscreen::new(gpu, s.width, s.height);
    let mut ring = Ring::new(gpu, s.width, s.height);

    let mut last_preview = Instant::now() - Duration::from_secs(1);
    let mut result = Ok(());
    let video_start = Instant::now();
    for n in 0..total {
        // Free the slot this frame will use by finishing the frame three back.
        let show = last_preview.elapsed() > Duration::from_millis(250);
        if let Some(frame) = ring.take(gpu, n, &mut writer, show) {
            if show {
                last_preview = Instant::now();
            }
            let done = frame + 1;
            let per = video_start.elapsed().as_secs_f64() / done as f64;
            let p = Progress {
                stage: Stage::Video,
                frame: done,
                total,
                elapsed: started.elapsed(),
                eta: Some(Duration::from_secs_f64(per * (total - done) as f64)),
                preview: if show { writer.last_frame() } else { None },
            };
            if !progress(&p) {
                result = Err(ExportError::Cancelled);
                break;
            }
        }
        if let Some(e) = writer.error() {
            result = Err(e);
            break;
        }
        let clock = FrameClock::at(s.fps, t0, n);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(
            gpu,
            &mut encoder,
            &target.view,
            FrameParams { time: clock.now(), frame: n, alpha: s.alpha },
        );
        ring.submit(gpu, encoder, &target, n);
    }
    if result.is_ok() {
        while ring.drain_one(gpu, &mut writer).is_some() {}
    }

    progress(&Progress {
        stage: Stage::Finishing,
        frame: total,
        total,
        elapsed: started.elapsed(),
        eta: None,
        preview: None,
    });
    let finished = writer.finish(result.is_err());
    match (result, finished) {
        (Err(e), _) => {
            remove_partial(s);
            Err(e)
        }
        (Ok(()), Err(e)) => {
            remove_partial(s);
            Err(e)
        }
        (Ok(()), Ok(())) => Ok(Summary {
            frames: total,
            seconds: t1 - t0,
            elapsed: started.elapsed(),
            output: s.output.clone(),
        }),
    }
}

fn io(path: &Path, e: std::io::Error) -> ExportError {
    ExportError::Io(format!("{}: {e}", path.display()))
}

/// A cancelled or failed render must not leave a corrupt file behind.
fn remove_partial(s: &Settings) {
    if s.preset != Preset::PngSequence {
        let _ = std::fs::remove_file(&s.output);
    }
}

struct TempFile(Option<PathBuf>);

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

// ── Readback ring ───────────────────────────────────────────────────────────

/// Three staging buffers. The CPU maps the one written two frames ago, long
/// finished, while the GPU works on the current frame — so neither waits on
/// the other. Two leaves no slack between copy, map, and hand-off.
struct Ring {
    slots: Vec<Slot>,
    width: u32,
    height: u32,
    next_out: u64,
}

struct Slot {
    buffer: wgpu::Buffer,
    frame: Option<u64>,
    ready: Arc<AtomicBool>,
    submission: Option<wgpu::SubmissionIndex>,
}

const RING: usize = 3;

impl Ring {
    fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let size = (padded_row(width) * height) as u64;
        let slots = (0..RING)
            .map(|_| Slot {
                buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("readback"),
                    size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                frame: None,
                ready: Arc::new(AtomicBool::new(false)),
                submission: None,
            })
            .collect();
        Self { slots, width, height, next_out: 0 }
    }

    fn submit(
        &mut self,
        gpu: &Gpu,
        mut encoder: wgpu::CommandEncoder,
        target: &Offscreen,
        frame: u64,
    ) {
        let slot = &mut self.slots[(frame as usize) % RING];
        debug_assert!(slot.frame.is_none());
        target.copy_to(&mut encoder, &slot.buffer);
        slot.submission = Some(gpu.queue.submit([encoder.finish()]));
        slot.ready.store(false, Ordering::Release);
        let ready = slot.ready.clone();
        slot.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
        slot.frame = Some(frame);
        // Nudge the GPU along without blocking.
        let _ = gpu.device.poll(wgpu::PollType::Poll);
    }

    /// Before rendering `frame`, finish whatever occupies its slot.
    fn take(&mut self, gpu: &Gpu, frame: u64, writer: &mut Writer, keep: bool) -> Option<u64> {
        let i = (frame as usize) % RING;
        self.slots[i].frame?;
        self.finish(gpu, i, writer, keep)
    }

    /// After the last frame, finish the rest in order.
    fn drain_one(&mut self, gpu: &Gpu, writer: &mut Writer) -> Option<u64> {
        let i = (self.next_out as usize) % RING;
        self.slots[i].frame?;
        self.finish(gpu, i, writer, false)
    }

    fn finish(&mut self, gpu: &Gpu, i: usize, writer: &mut Writer, keep: bool) -> Option<u64> {
        let slot = &mut self.slots[i];
        let frame = slot.frame.take()?;
        if !slot.ready.load(Ordering::Acquire) {
            let _ = gpu.device.poll(wgpu::PollType::Wait {
                submission_index: slot.submission.take(),
                timeout: None,
            });
        }
        {
            let data = slot.buffer.get_mapped_range(..).expect("mapped readback");
            let mut out = writer.buffer();
            unpad(&data, self.width, self.height, &mut out);
            writer.send(out, keep);
        }
        slot.buffer.unmap();
        self.next_out = frame + 1;
        Some(frame)
    }
}

// ── Writer thread ───────────────────────────────────────────────────────────

enum Sink {
    Ffmpeg(std::process::Child),
    Png(PathBuf),
}

/// Encoding runs beside rendering: frames go to a thread that writes them,
/// and their buffers come back to be reused, so the steady state allocates
/// nothing and the GPU never waits on the encoder beyond a small queue.
struct Writer {
    tx: Option<mpsc::SyncSender<Vec<u8>>>,
    recycled: mpsc::Receiver<Vec<u8>>,
    handle: Option<std::thread::JoinHandle<Result<(), ExportError>>>,
    failed: Arc<std::sync::Mutex<Option<String>>>,
    /// Copy of the latest frame, for the thumbnail. Reuses its allocation.
    last: Vec<u8>,
    kill: Arc<AtomicBool>,
}

impl Writer {
    fn start(sink: Sink, width: u32, height: u32) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(4);
        let (back_tx, recycled) = mpsc::channel();
        let failed = Arc::new(std::sync::Mutex::new(None));
        let kill = Arc::new(AtomicBool::new(false));
        let (f2, k2) = (failed.clone(), kill.clone());
        let handle = std::thread::spawn(move || -> Result<(), ExportError> {
            let mut n = 0u64;
            let fail = |m: String| {
                *f2.lock().unwrap() = Some(m.clone());
                ExportError::Io(m)
            };
            match sink {
                Sink::Ffmpeg(mut child) => {
                    let mut stdin = child.stdin.take().expect("piped stdin");
                    let stderr = child.stderr.take().expect("piped stderr");
                    // Drain stderr on its own thread so a chatty ffmpeg can
                    // never block on a full pipe.
                    let log = std::thread::spawn(move || {
                        let mut s = String::new();
                        let _ = std::io::Read::read_to_string(
                            &mut std::io::BufReader::new(stderr),
                            &mut s,
                        );
                        s
                    });
                    let mut write_err = None;
                    for frame in rx {
                        if write_err.is_none()
                            && let Err(e) = stdin.write_all(&frame)
                        {
                            write_err = Some(e);
                        }
                        let _ = back_tx.send(frame);
                    }
                    drop(stdin);
                    if k2.load(Ordering::Acquire) {
                        let _ = child.kill();
                    }
                    let status = child.wait().map_err(|e| fail(e.to_string()))?;
                    let log = log.join().unwrap_or_default();
                    if k2.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    if !status.success() || write_err.is_some() {
                        // ffmpeg's own words, verbatim: far more useful than ours.
                        return Err(ExportError::Ffmpeg(
                            fail(format!(
                                "ffmpeg failed ({status}):\n{}",
                                if log.trim().is_empty() {
                                    "no output".into()
                                } else {
                                    log.trim().to_string()
                                }
                            ))
                            .to_string(),
                        ));
                    }
                    Ok(())
                }
                Sink::Png(dir) => {
                    for frame in rx {
                        let path = dir.join(format!("frame_{n:06}.png"));
                        n += 1;
                        let res =
                            std::fs::File::create(&path).map_err(|e| e.to_string()).and_then(|f| {
                                let mut enc =
                                    png::Encoder::new(std::io::BufWriter::new(f), width, height);
                                enc.set_color(png::ColorType::Rgba);
                                enc.set_depth(png::BitDepth::Eight);
                                enc.set_compression(png::Compression::Fast);
                                enc.write_header()
                                    .and_then(|mut w| w.write_image_data(&frame))
                                    .map_err(|e| e.to_string())
                            });
                        let _ = back_tx.send(frame);
                        if let Err(e) = res {
                            return Err(fail(format!("{}: {e}", path.display())));
                        }
                    }
                    Ok(())
                }
            }
        });
        Self { tx: Some(tx), recycled, handle: Some(handle), failed, last: Vec::new(), kill }
    }

    fn buffer(&self) -> Vec<u8> {
        self.recycled.try_recv().unwrap_or_default()
    }

    fn send(&mut self, frame: Vec<u8>, keep: bool) {
        if keep {
            self.last.clear();
            self.last.extend_from_slice(&frame);
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(frame);
        }
    }

    fn last_frame(&self) -> Option<&[u8]> {
        (!self.last.is_empty()).then_some(self.last.as_slice())
    }

    fn error(&self) -> Option<ExportError> {
        self.failed.lock().unwrap().clone().map(ExportError::Io)
    }

    fn finish(mut self, cancel: bool) -> Result<(), ExportError> {
        self.kill.store(cancel, Ordering::Release);
        drop(self.tx.take());
        match self.handle.take().map(|h| h.join()) {
            Some(Ok(r)) => r,
            _ => Err(ExportError::Io("encoder thread panicked".into())),
        }
    }
}
