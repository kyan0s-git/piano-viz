//! The render dialog and the progress window.

use super::{ACCENT, Action, Cx, MUTED_TEXT, UiState};
use egui::{Color32, RichText};
use pv_core::Rational;
use pv_export::{Job, Preset, Settings};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Mp4,
    H265,
    ProRes,
    WebM,
    Lossless,
    Png,
}

const FORMATS: &[(Format, &str)] = &[
    (Format::Mp4, "MP4 (H.264) — plays everywhere"),
    (Format::H265, "MP4 (H.265) — smaller files"),
    (Format::ProRes, "ProRes 4444 — for editors, with alpha"),
    (Format::WebM, "WebM (VP9) — with alpha"),
    (Format::Lossless, "Lossless (FFV1)"),
    (Format::Png, "PNG sequence — no ffmpeg needed"),
];

const SIZES: &[((u32, u32), &str)] = &[
    ((1280, 720), "720p"),
    ((1920, 1080), "1080p"),
    ((2560, 1440), "1440p"),
    ((3840, 2160), "4K"),
    ((1080, 1920), "Vertical 1080×1920"),
    ((1080, 1080), "Square 1080×1080"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Range {
    Whole,
    Loop,
    Custom,
}

pub struct ExportForm {
    width: u32,
    height: u32,
    fps: Rational,
    format: Format,
    crf: u8,
    ss: u32,
    range: Range,
    from: f64,
    to: f64,
    alpha: bool,
    audio: bool,
    output: String,
    /// Validation result for the current settings, and what it was for.
    check: Option<(String, Result<String, String>)>,
}

impl Default for ExportForm {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: Rational::new(60, 1),
            format: Format::Mp4,
            crf: 18,
            ss: 2,
            range: Range::Whole,
            from: 0.0,
            to: 10.0,
            alpha: false,
            audio: true,
            output: String::new(),
            check: None,
        }
    }
}

impl ExportForm {
    /// The preview frames to the export's shape.
    pub fn frame_sync(&self, s: &mut crate::session::Session) {
        s.frame_size = (self.width.max(16), self.height.max(16));
    }

    fn preset(&self) -> Preset {
        match self.format {
            Format::Mp4 => Preset::H264 { crf: self.crf },
            Format::H265 => Preset::H265 { crf: self.crf },
            Format::ProRes => Preset::ProRes4444,
            Format::WebM => Preset::Vp9Alpha,
            Format::Lossless => Preset::Ffv1,
            Format::Png => Preset::PngSequence,
        }
    }

    fn settings(&self, end_default: Option<(f64, f64)>) -> Settings {
        let (start, end) = match (self.range, end_default) {
            (Range::Loop, Some((a, b))) => (a, Some(b)),
            (Range::Custom, _) => (self.from, Some(self.to.max(self.from + 0.1))),
            _ => (0.0, None),
        };
        Settings {
            width: self.width,
            height: self.height,
            fps: self.fps,
            supersample: self.ss,
            start,
            end,
            tail: None,
            preset: self.preset(),
            alpha: self.alpha && self.preset().has_alpha(),
            output: PathBuf::from(&self.output),
            ffmpeg: None,
            audio: self.audio,
            sample_rate: 48_000,
        }
    }
}

fn default_dir() -> PathBuf {
    let home =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    match home {
        Some(h) if h.join("Videos").is_dir() => h.join("Videos"),
        Some(h) if h.join("Movies").is_dir() => h.join("Movies"),
        Some(h) => h,
        None => PathBuf::from("."),
    }
}

fn with_extension(path: &str, preset: Preset) -> String {
    let p = Path::new(path);
    let ext = preset.extension();
    if ext.is_empty() {
        let stem = p.with_extension("");
        let s = stem.display().to_string();
        if s.ends_with("_frames") { s } else { format!("{s}_frames") }
    } else {
        let s = p.display().to_string();
        let base =
            s.strip_suffix("_frames").map(PathBuf::from).unwrap_or_else(|| p.with_extension(""));
        base.with_extension(ext).display().to_string()
    }
}

pub fn dialog(ctx: &egui::Context, st: &mut UiState, cx: &mut Cx) {
    if !st.export_open || cx.export.is_some() {
        return;
    }
    let f = &mut st.export;
    if f.output.is_empty() {
        let stem: String = cx
            .session
            .score_name
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_".contains(*c))
            .collect();
        let stem =
            if stem.trim().is_empty() { "piano-viz".into() } else { stem.trim().replace(' ', "_") };
        f.output = with_extension(&default_dir().join(stem).display().to_string(), f.preset());
    }
    let mut open = true;
    let mut start = false;
    egui::Window::new("Export video")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(440.0)
        .show(ctx, |ui| {
            egui::Grid::new("export").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Format");
                let before = f.format;
                egui::ComboBox::from_id_salt("fmt")
                    .width(280.0)
                    .selected_text(FORMATS.iter().find(|(x, _)| *x == f.format).map_or("", |(_, n)| n))
                    .show_ui(ui, |ui| {
                        for (x, n) in FORMATS {
                            ui.selectable_value(&mut f.format, *x, *n);
                        }
                    });
                if before != f.format {
                    f.output = with_extension(&f.output, f.preset());
                }
                ui.end_row();

                ui.label("Size");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("size")
                        .width(150.0)
                        .selected_text(SIZES.iter().find(|(s, _)| *s == (f.width, f.height)).map_or("Custom", |(_, n)| n))
                        .show_ui(ui, |ui| {
                            for ((w, h), n) in SIZES {
                                if ui.selectable_label((f.width, f.height) == (*w, *h), *n).clicked() {
                                    (f.width, f.height) = (*w, *h);
                                }
                            }
                        });
                    ui.add(egui::DragValue::new(&mut f.width).range(16..=8192).suffix(" w"));
                    ui.add(egui::DragValue::new(&mut f.height).range(16..=8192).suffix(" h"));
                });
                ui.end_row();

                ui.label("Frame rate");
                egui::ComboBox::from_id_salt("fps").selected_text(format!("{:.2}", f.fps.as_f64()).replace(".00", "")).show_ui(ui, |ui| {
                    for (r, n) in [
                        (Rational::new(24, 1), "24"),
                        (Rational::new(25, 1), "25"),
                        (Rational::new(30, 1), "30"),
                        (Rational::new(50, 1), "50"),
                        (Rational::new(60, 1), "60"),
                        (Rational::new(30000, 1001), "29.97"),
                        (Rational::new(60000, 1001), "59.94"),
                        (Rational::new(120, 1), "120"),
                    ] {
                        ui.selectable_value(&mut f.fps, r, n);
                    }
                });
                ui.end_row();

                if matches!(f.format, Format::Mp4 | Format::H265) {
                    ui.label("Quality").on_hover_text("CRF: lower is better and larger. 18 is visually lossless for most footage.");
                    ui.add(egui::Slider::new(&mut f.crf, 12..=30).text("CRF (lower = better)"));
                    ui.end_row();
                }

                ui.label("Smoothing").on_hover_text("Supersampling: renders larger and scales down. Slower, cleaner edges.");
                ui.horizontal(|ui| {
                    for s in 1..=4 {
                        ui.selectable_value(&mut f.ss, s, format!("{s}×"));
                    }
                });
                ui.end_row();

                ui.label("Range");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut f.range, Range::Whole, "Whole piece");
                        ui.add_enabled_ui(cx.session.loop_range.is_some(), |ui| {
                            ui.selectable_value(&mut f.range, Range::Loop, "Loop region");
                        });
                        ui.selectable_value(&mut f.range, Range::Custom, "Custom");
                    });
                    if f.range == Range::Custom {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut f.from).speed(0.1).range(0.0..=f64::MAX).suffix(" s").prefix("from "));
                            ui.add(egui::DragValue::new(&mut f.to).speed(0.1).range(0.0..=f64::MAX).suffix(" s").prefix("to "));
                        });
                    }
                });
                ui.end_row();

                ui.label("Options");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.audio, "Audio");
                    ui.add_enabled(f.preset().has_alpha(), egui::Checkbox::new(&mut f.alpha, "Transparent background"))
                        .on_disabled_hover_text("Needs ProRes, WebM, or a PNG sequence");
                });
                ui.end_row();

                ui.label("Save to");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.output).desired_width(250.0));
                    if ui.button("…").clicked() {
                        let picked = if f.format == Format::Png {
                            rfd::FileDialog::new().pick_folder()
                        } else {
                            rfd::FileDialog::new().add_filter("Video", &[f.preset().extension()]).set_file_name(
                                Path::new(&f.output).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
                            ).save_file()
                        };
                        if let Some(p) = picked {
                            f.output = p.display().to_string();
                        }
                    }
                });
                ui.end_row();
            });

            // Estimate and validate as settings change, not after a long wait.
            let settings = f.settings(cx.session.loop_range);
            let (t0, t1) = pv_export::range(&cx.session.score, &cx.session.design, &settings);
            let frames = pv_export::frame_count(t1 - t0, settings.fps);
            let key = format!("{:?}{}{}{}{}", settings.preset, settings.width, settings.height, settings.alpha, settings.output.display());
            if f.check.as_ref().is_none_or(|(k, _)| *k != key) {
                let r = pv_export::validate(&settings)
                    .map(|bin| bin.map_or("Ready — no ffmpeg needed for PNGs".into(), |b| format!("Ready — ffmpeg at {}", b.display())))
                    .map_err(|e| e.to_string());
                f.check = Some((key, r));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{:.1} s · {frames} frames", t1 - t0)).color(MUTED_TEXT));
            let ok = match &f.check {
                Some((_, Ok(msg))) => {
                    ui.label(RichText::new(msg).small().color(MUTED_TEXT));
                    true
                }
                Some((_, Err(e))) => {
                    ui.label(RichText::new(e).color(Color32::from_rgb(0xff, 0x9a, 0x8a)));
                    false
                }
                None => false,
            };
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let go = ui.add_enabled(ok && !f.output.trim().is_empty(), egui::Button::new(RichText::new("Export").color(Color32::BLACK)).fill(ACCENT));
                if go.clicked() {
                    start = true;
                }
                ui.label(RichText::new("Preview pauses while exporting.").small().color(MUTED_TEXT));
            });
        });
    if start {
        let s = &*cx.session;
        let job = Job {
            score: s.score.clone(),
            design: s.design.clone(),
            bundle_dir: s.bundle_dir.clone(),
            font: cx.transport.font().cloned().unwrap_or_else(pv_audio::builtin_font),
            track_visible: s.shown(),
            track_muted: s.silenced(),
            sequence: cx.transport.options(),
            volume: cx.transport.volume(),
            settings: st.export.settings(s.loop_range),
        };
        cx.actions.push(Action::StartExport(Box::new(job)));
        st.export_open = false;
    } else if !open {
        st.export_open = false;
    }
}

pub fn progress(ctx: &egui::Context, thumb: &mut Option<egui::TextureHandle>, cx: &mut Cx) {
    let Some(run) = cx.export.as_mut() else {
        *thumb = None;
        return;
    };
    if run.thumbnail_fresh
        && let Some(t) = &run.thumbnail
    {
        run.thumbnail_fresh = false;
        let img = egui::ColorImage::from_rgba_unmultiplied(
            [t.width as usize, t.height as usize],
            &t.rgba,
        );
        match thumb {
            Some(h) => h.set(img, egui::TextureOptions::LINEAR),
            None => {
                *thumb = Some(ctx.load_texture("export-thumb", img, egui::TextureOptions::LINEAR))
            }
        }
    }
    egui::Window::new("Exporting")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(480.0);
            if let Some(h) = thumb {
                let size = h.size_vec2();
                ui.image((h.id(), size * (480.0 / size.x.max(1.0))));
            }
            let stage = match run.stage {
                pv_export::Stage::Audio => "Rendering audio",
                pv_export::Stage::Video => "Rendering video",
                pv_export::Stage::Finishing => "Finishing",
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(stage).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}%", run.fraction() * 100.0));
                });
            });
            ui.add(egui::ProgressBar::new(run.fraction()).fill(ACCENT).desired_height(8.0));
            let eta = run.eta.map_or(String::new(), |e| {
                format!(" · about {}:{:02} left", e.as_secs() / 60, e.as_secs() % 60)
            });
            ui.label(
                RichText::new(format!("frame {} of {}{eta}", run.frame, run.total))
                    .color(MUTED_TEXT),
            );
            ui.label(RichText::new(run.output.display().to_string()).small().color(MUTED_TEXT));
            if ui.button("Cancel").clicked() {
                run.cancel();
            }
        });
}
