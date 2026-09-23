//! `pv`: piano-viz without a window. Renders videos and stills, batch
//! processes folders, and inspects MIDI files. Same engine as the app.

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pv_core::{Rational, Score};
use pv_export::{Job, Preset, Settings, Stage};
use pv_render::{FrameParams, Gpu, Offscreen, Renderer, render_image};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "pv", version, about = "piano-viz: render piano MIDI to video", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a MIDI file to video (or a PNG sequence).
    Render {
        /// MIDI file, or `demo` for the built-in piece.
        midi: String,
        /// Output file (a directory for --format png). Defaults beside the MIDI.
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        opts: RenderOpts,
    },
    /// Render one frame to PNG.
    Still {
        midi: String,
        /// Seconds into the song.
        #[arg(short = 't', long, default_value_t = 10.0)]
        at: f64,
        #[arg(short, long, default_value = "still.png")]
        output: PathBuf,
        #[command(flatten)]
        look: LookOpts,
        /// Transparent background.
        #[arg(long)]
        alpha: bool,
    },
    /// Render every MIDI file in a folder.
    Batch {
        dir: PathBuf,
        /// Where the videos go.
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        opts: RenderOpts,
    },
    /// Show what's in a MIDI file.
    Inspect { midi: String },
    /// List the built-in Designs.
    Designs,
    /// Write the built-in demo piece as a MIDI file.
    Demo {
        #[arg(short, long, default_value = "prelude.mid")]
        output: PathBuf,
    },
}

#[derive(Args, Clone)]
struct LookOpts {
    /// Built-in Design name, a design .toml, or a bundle directory.
    #[arg(short, long, default_value = "ember-classic")]
    design: String,
    /// Output size, WIDTHxHEIGHT.
    #[arg(short, long, default_value = "1920x1080", value_parser = parse_size)]
    size: (u32, u32),
    /// Supersampling factor, 1-4. Higher is smoother and slower.
    #[arg(long, default_value_t = 2)]
    ss: u32,
    /// Hide tracks by number (1-based), e.g. --hide 3 --hide 4.
    #[arg(long)]
    hide: Vec<usize>,
}

#[derive(Args, Clone)]
struct RenderOpts {
    #[command(flatten)]
    look: LookOpts,
    /// Frames per second: 24, 30, 60, 29.97, 59.94, or N/D.
    #[arg(long, default_value = "60", value_parser = parse_fps)]
    fps: Rational,
    #[arg(short, long, value_enum, default_value_t = Format::Mp4)]
    format: Format,
    /// Quality for mp4/h265: lower is better and bigger.
    #[arg(long, default_value_t = 18)]
    crf: u8,
    /// Transparent background (prores, webm, png).
    #[arg(long)]
    alpha: bool,
    /// Start, in seconds.
    #[arg(long, default_value_t = 0.0)]
    from: f64,
    /// End, in seconds. Default: last note plus the tail.
    #[arg(long)]
    to: Option<f64>,
    /// Seconds after the last note, for effects to fade out.
    #[arg(long)]
    tail: Option<f64>,
    /// Leave out the audio track.
    #[arg(long)]
    no_audio: bool,
    /// Use this SoundFont (.sf2) instead of the built-in piano.
    #[arg(long)]
    soundfont: Option<PathBuf>,
    /// Play program changes as written instead of forcing piano.
    #[arg(long)]
    all_instruments: bool,
    /// Path to ffmpeg, if it isn't on PATH.
    #[arg(long)]
    ffmpeg: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// H.264 MP4: plays everywhere, YouTube-ready.
    Mp4,
    /// H.265 MP4: smaller files.
    H265,
    /// ProRes 4444 MOV, with alpha: for video editors.
    Prores,
    /// VP9 WebM, with alpha.
    Webm,
    /// FFV1 MKV: lossless.
    Lossless,
    /// Numbered PNG files plus audio.wav. No ffmpeg needed.
    Png,
}

impl Format {
    fn preset(self, crf: u8) -> Preset {
        match self {
            Self::Mp4 => Preset::H264 { crf },
            Self::H265 => Preset::H265 { crf },
            Self::Prores => Preset::ProRes4444,
            Self::Webm => Preset::Vp9Alpha,
            Self::Lossless => Preset::Ffv1,
            Self::Png => Preset::PngSequence,
        }
    }
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once(['x', 'X']).ok_or("expected WIDTHxHEIGHT, e.g. 1920x1080")?;
    let w = w.trim().parse().map_err(|_| "bad width")?;
    let h = h.trim().parse().map_err(|_| "bad height")?;
    Ok((w, h))
}

fn parse_fps(s: &str) -> Result<Rational, String> {
    Rational::parse_fps(s).ok_or_else(|| format!("unsupported frame rate {s:?}"))
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Render { midi, output, opts } => {
            let gpu = gpu()?;
            let out = output.unwrap_or_else(|| default_output(&midi, opts.format));
            render(&gpu, &midi, &out, &opts)
        }
        Command::Still { midi, at, output, look, alpha } => still(&midi, at, &output, &look, alpha),
        Command::Batch { dir, out, opts } => batch(&dir, &out, &opts),
        Command::Inspect { midi } => inspect(&midi),
        Command::Designs => {
            for (id, src) in pv_design::BUILTIN {
                let d = pv_design::from_toml(src)?.design;
                println!("{id:<15} {}", d.meta.description);
            }
            Ok(())
        }
        Command::Demo { output } => {
            std::fs::write(&output, pv_midi::demo::prelude())
                .with_context(|| output.display().to_string())?;
            println!("wrote {}", output.display());
            Ok(())
        }
    }
}

fn gpu() -> Result<Gpu> {
    let gpu = Gpu::headless().context("no GPU available")?;
    eprintln!("using {}", gpu.describe());
    Ok(gpu)
}

fn load_score(midi: &str) -> Result<Score> {
    let score = if midi == "demo" {
        pv_midi::parse(&pv_midi::demo::prelude())?
    } else {
        pv_midi::load(midi)?
    };
    for w in &score.warnings {
        eprintln!("note: {w}");
    }
    Ok(score)
}

fn load_design(spec: &str) -> Result<(pv_design::Design, Option<PathBuf>)> {
    if let Some((_, src)) = pv_design::BUILTIN.iter().find(|(id, _)| *id == spec) {
        return Ok((pv_design::from_toml(src)?.design, None));
    }
    let path = Path::new(spec);
    if !path.exists() {
        let names: Vec<_> = pv_design::BUILTIN.iter().map(|(id, _)| *id).collect();
        bail!("no Design {spec:?}: not a file, and not one of {}", names.join(", "));
    }
    let loaded = pv_design::load(path)?;
    for w in &loaded.warnings {
        eprintln!("design: {w}");
    }
    Ok((loaded.design, loaded.bundle_dir))
}

fn visibility(score: &Score, hide: &[usize]) -> Vec<bool> {
    (0..score.tracks.len()).map(|i| !hide.contains(&(i + 1))).collect()
}

fn default_output(midi: &str, format: Format) -> PathBuf {
    let stem = if midi == "demo" {
        "demo".into()
    } else {
        Path::new(midi).file_stem().map_or("out".into(), |s| s.to_string_lossy().into_owned())
    };
    let dir = if midi == "demo" {
        PathBuf::new()
    } else {
        Path::new(midi).parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let ext = format.preset(18).extension();
    if ext.is_empty() {
        dir.join(format!("{stem}_frames"))
    } else {
        dir.join(format!("{stem}.{ext}"))
    }
}

fn render(gpu: &Gpu, midi: &str, out: &Path, opts: &RenderOpts) -> Result<()> {
    let score = Arc::new(load_score(midi)?);
    let (design, bundle_dir) = load_design(&opts.look.design)?;
    let font = match &opts.soundfont {
        Some(p) => pv_audio::load_font(p)?,
        None => pv_audio::builtin_font(),
    };
    let visible = visibility(&score, &opts.look.hide);
    let job = Job {
        track_visible: visible.clone(),
        track_muted: visible.iter().map(|v| !v).collect(),
        sequence: pv_audio::SequenceOptions {
            force_piano: !opts.all_instruments,
            skip_percussion: !pv_audio::has_drums(&font),
        },
        volume: 1.0,
        score: score.clone(),
        design,
        bundle_dir,
        font,
        settings: Settings {
            width: opts.look.size.0,
            height: opts.look.size.1,
            fps: opts.fps,
            supersample: opts.look.ss,
            start: opts.from,
            end: opts.to,
            tail: opts.tail,
            preset: opts.format.preset(opts.crf),
            alpha: opts.alpha,
            output: out.to_path_buf(),
            ffmpeg: opts.ffmpeg.clone(),
            audio: !opts.no_audio,
            sample_rate: 48_000,
        },
    };
    let (t0, t1) = pv_export::range(&score, &job.design, &job.settings);
    eprintln!(
        "rendering {:.1}s at {}x{} {:.2} fps -> {}",
        t1 - t0,
        job.settings.width,
        job.settings.height,
        job.settings.fps.as_f64(),
        out.display()
    );
    let mut last = std::time::Instant::now();
    let summary = pv_export::export(gpu, &job, |p| {
        if last.elapsed().as_millis() >= 200 || p.frame == p.total {
            last = std::time::Instant::now();
            let stage = match p.stage {
                Stage::Audio => "audio",
                Stage::Video => "video",
                Stage::Finishing => "finishing",
            };
            let eta = p.eta.map_or(String::new(), |e| {
                format!("  eta {}:{:02}", e.as_secs() / 60, e.as_secs() % 60)
            });
            eprint!(
                "\r  {stage:<9} {:>6}/{} {:5.1}%{eta}    ",
                p.frame,
                p.total,
                p.frame as f64 * 100.0 / p.total as f64
            );
            let _ = std::io::stderr().flush();
        }
        true
    })?;
    let secs = summary.elapsed.as_secs_f64();
    eprintln!(
        "\ndone: {} frames in {secs:.1}s ({:.1} fps, {:.2}x real time) -> {}",
        summary.frames,
        summary.frames as f64 / secs,
        summary.seconds / secs,
        summary.output.display()
    );
    Ok(())
}

fn still(midi: &str, at: f64, output: &Path, look: &LookOpts, alpha: bool) -> Result<()> {
    let gpu = gpu()?;
    let score = Arc::new(load_score(midi)?);
    let (design, bundle_dir) = load_design(&look.design)?;
    let (w, h) = look.size;
    let mut r = Renderer::new(&gpu, pv_render::CAPTURE_FORMAT, w, h, look.ss);
    r.set_score(&gpu, score.clone());
    for warn in r.set_design(&gpu, &design, bundle_dir.as_deref()) {
        eprintln!("design: {warn}");
    }
    r.set_track_visibility(&gpu, &visibility(&score, &look.hide));
    let img = render_image(
        &gpu,
        &mut r,
        &Offscreen::new(&gpu, w, h),
        FrameParams { time: at, frame: 0, alpha },
    );
    let file = std::io::BufWriter::new(
        std::fs::File::create(output).with_context(|| output.display().to_string())?,
    );
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img)?;
    println!("wrote {}", output.display());
    Ok(())
}

fn batch(dir: &Path, out: &Path, opts: &RenderOpts) -> Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| dir.display().to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mid") || e.eq_ignore_ascii_case("midi"))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("no .mid files in {}", dir.display());
    }
    std::fs::create_dir_all(out)?;
    let gpu = gpu()?;
    let mut failed = 0;
    for (i, f) in files.iter().enumerate() {
        eprintln!("[{}/{}] {}", i + 1, files.len(), f.display());
        let name = default_output(&f.to_string_lossy(), opts.format);
        let target = out.join(name.file_name().unwrap_or_default());
        // One bad file shouldn't stop the batch.
        if let Err(e) = render(&gpu, &f.to_string_lossy(), &target, opts) {
            eprintln!("  failed: {e:#}");
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} files failed", files.len());
    }
    Ok(())
}

fn inspect(midi: &str) -> Result<()> {
    let s = load_score(midi)?;
    let d = s.duration;
    println!("title:     {}", s.title.as_deref().unwrap_or("-"));
    println!("duration:  {}:{:04.1}", (d / 60.0) as u32, d % 60.0);
    println!("notes:     {}", s.notes.len());
    if let Some((lo, hi)) = s.notes.pitch_range() {
        println!("range:     {} - {}", note_name(lo), note_name(hi));
    }
    println!("tempo:     {:.1} BPM at start", s.tempo.bpm_at(0.0));
    if let Some(ts) = s.tempo.time_signatures().first() {
        println!("time sig:  {}/{}", ts.numerator, ts.denominator);
    }
    println!("tracks:");
    for (i, t) in s.tracks.iter().enumerate() {
        let range = t
            .pitch_range
            .map_or(String::new(), |(a, b)| format!("{}-{}", note_name(a), note_name(b)));
        println!(
            "  {:>2}  {:<28} {:>6} notes  {:<9} {}",
            i + 1,
            t.label(i),
            t.note_count,
            range,
            t.instrument.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

fn note_name(p: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[(p % 12) as usize], p as i32 / 12 - 1)
}
