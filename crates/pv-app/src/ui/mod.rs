//! The interface: menu, track panel, Design inspector, transport and
//! timeline around a letterboxed preview. See docs/10-ui-ux.md.

mod export;
mod inspector;

pub use export::ExportForm;

use crate::exporter::ExportRun;
use crate::session::{DesignSource, Session};
use crate::transport::Transport;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use pv_render::Renderer;
use std::path::PathBuf;

/// Warm accent, echoing the default Design.
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x8a, 0x3d);
const MUTED_TEXT: Color32 = Color32::from_rgb(0x8c, 0x8a, 0x86);

/// Things the UI asks for that need more than a field write.
pub enum Action {
    OpenMidi,
    OpenDesign,
    SaveDesign,
    OpenSoundFont,
    BuiltinPiano,
    Builtin(&'static str),
    Seek(f64),
    StartExport(Box<pv_export::Job>),
    Presentation,
    ExitPresentation,
    Quit,
    Live(bool),
    LiveConnect(String),
    LiveDisconnect,
    NoteOn(u8),
    NoteOff(u8),
    Pedal(bool),
    Record(bool),
    SaveMidi,
    LowLatency(bool),
}

pub struct UiState {
    pub panels: bool,
    pub hud: bool,
    pub export_open: bool,
    pub hint_dismissed: bool,
    pub repaint_now: bool,
    /// When egui wants the next frame, if it asked for one.
    pub repaint_at: Option<std::time::Instant>,
    pub search: String,
    pub export: ExportForm,
    pub midi_path: Option<PathBuf>,
    thumb: Option<egui::TextureHandle>,
    /// Key held down by the mouse on the preview's keyboard, in live mode.
    mouse_key: Option<u8>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            panels: true,
            hud: false,
            export_open: false,
            hint_dismissed: false,
            repaint_now: true,
            repaint_at: None,
            search: String::new(),
            export: ExportForm::default(),
            midi_path: None,
            thumb: None,
            mouse_key: None,
        }
    }
}

/// What the UI can see and touch this frame.
pub struct Cx<'a> {
    pub session: &'a mut Session,
    pub transport: &'a mut Transport,
    pub renderer: &'a Renderer,
    pub export: &'a mut Option<ExportRun>,
    pub preview_tex: Option<egui::TextureId>,
    /// Out: the preview's size in physical pixels, for next frame.
    pub preview_px: &'a mut (u32, u32),
    pub frame_times: &'a [f32],
    pub gpu_name: String,
    pub presentation: bool,
    pub actions: &'a mut Vec<Action>,
    pub time: f64,
    pub live: &'a mut crate::live::Live,
}

pub fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        v.panel_fill = Color32::from_rgb(0x15, 0x15, 0x17);
        v.window_fill = Color32::from_rgb(0x1b, 0x1b, 0x1e);
        v.extreme_bg_color = Color32::from_rgb(0x0e, 0x0e, 0x10);
        v.faint_bg_color = Color32::from_rgb(0x1d, 0x1d, 0x20);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        v.slider_trailing_fill = true;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x28, 0x28, 0x2c));
        v.window_corner_radius = CornerRadius::same(6);
        s.spacing.item_spacing = vec2(8.0, 5.0);
        s.spacing.slider_width = 130.0;
    });
}

/// Keyboard shortcuts (docs/10-ui-ux.md). Read through egui so Ctrl-letter
/// combinations behave the same on every platform, and so typing in a text
/// field never triggers one.
fn shortcuts(ctx: &egui::Context, st: &mut UiState, cx: &mut Cx) {
    use egui::{Key, Modifiers};
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    if cx.live.active {
        live_keys(ctx, st, cx);
        return;
    }
    let t = cx.time;
    let beat = 60.0 / cx.session.score.tempo.bpm_at(t);
    let dur = cx.session.score.duration;
    ctx.input_mut(|i| {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if i.consume_key(cmd_shift, Key::Z) || i.consume_key(cmd, Key::Y) {
            cx.session.redo();
        }
        if i.consume_key(cmd, Key::Z) {
            cx.session.undo();
        }
        if i.consume_key(cmd, Key::O) {
            cx.actions.push(Action::OpenMidi);
        }
        if i.consume_key(cmd, Key::R) {
            st.export_open = true;
        }
        if i.consume_key(cmd, Key::L) {
            cx.session.looping = !cx.session.looping;
        }
        let none = Modifiers::NONE;
        if i.consume_key(none, Key::Space) {
            cx.transport.toggle();
            st.hint_dismissed = true;
        }
        if i.consume_key(Modifiers::SHIFT, Key::ArrowLeft) {
            cx.actions.push(Action::Seek(t - beat * 4.0));
        }
        if i.consume_key(Modifiers::SHIFT, Key::ArrowRight) {
            cx.actions.push(Action::Seek(t + beat * 4.0));
        }
        if i.consume_key(none, Key::ArrowLeft) {
            cx.actions.push(Action::Seek(t - beat));
        }
        if i.consume_key(none, Key::ArrowRight) {
            cx.actions.push(Action::Seek(t + beat));
        }
        if i.consume_key(none, Key::Home) {
            cx.actions.push(Action::Seek(0.0));
        }
        if i.consume_key(none, Key::End) {
            cx.actions.push(Action::Seek(dur));
        }
        if i.consume_key(none, Key::J) {
            cx.actions.push(Action::Seek(t - 5.0));
        }
        if i.consume_key(none, Key::K) {
            cx.transport.pause();
        }
        if i.consume_key(none, Key::L) {
            cx.transport.play();
        }
        if i.consume_key(none, Key::OpenBracket) {
            let end = cx.session.loop_range.map_or(dur, |r| r.1).max(t + 0.5);
            cx.session.loop_range = Some((t, end));
            cx.session.looping = true;
        }
        if i.consume_key(none, Key::CloseBracket) {
            let start = cx.session.loop_range.map_or(0.0, |r| r.0).min(t - 0.5).max(0.0);
            cx.session.loop_range = Some((start, t));
            cx.session.looping = true;
        }
        if i.consume_key(none, Key::F3) {
            st.hud = !st.hud;
        }
        if i.consume_key(none, Key::F11) {
            cx.actions.push(Action::Presentation);
        }
        if i.consume_key(none, Key::Escape) {
            cx.actions.push(Action::ExitPresentation);
            st.export_open = false;
        }
        if i.consume_key(none, Key::Tab) {
            st.panels = !st.panels;
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (n, k) in digits.into_iter().enumerate() {
            if i.consume_key(none, k) {
                let track = cx.session.score.visible_tracks().nth(n).map(|(i, _)| i);
                cx.session.solo = if cx.session.solo == track { None } else { track };
                cx.session.tracks_dirty = true;
            }
        }
    });
}

/// Live mode: the computer keyboard is a piano. Home row white keys, the
/// row above black keys, Z/X shift the octave, Space is the sustain pedal.
fn live_keys(ctx: &egui::Context, st: &mut UiState, cx: &mut Cx) {
    use egui::{Event, Key};
    let focused = ctx.input(|i| i.focused);
    if !focused {
        // Releases never arrive for keys held while the window loses focus.
        for k in 0..128u8 {
            if std::mem::take(&mut cx.live.typed[k as usize]) {
                cx.actions.push(Action::NoteOff(k));
            }
        }
        return;
    }
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let Event::Key { key, pressed, repeat, modifiers, .. } = *e else { return true };
            if modifiers.command || modifiers.alt {
                return true;
            }
            if repeat {
                return false;
            }
            match key {
                Key::Escape if pressed => cx.actions.push(Action::Live(false)),
                Key::F3 if pressed => st.hud = !st.hud,
                Key::F11 if pressed => cx.actions.push(Action::Presentation),
                Key::Z if pressed => cx.live.octave = cx.live.octave.saturating_sub(12).max(12),
                Key::X if pressed => cx.live.octave = (cx.live.octave + 12).min(96),
                Key::Space => cx.actions.push(Action::Pedal(pressed)),
                _ => {
                    let Some(off) = crate::live::Live::typing_offset(key) else { return true };
                    let k = (cx.live.octave + off).min(127);
                    let held = &mut cx.live.typed[k as usize];
                    if pressed && !*held {
                        *held = true;
                        cx.actions.push(Action::NoteOn(k));
                    } else if !pressed && *held {
                        *held = false;
                        cx.actions.push(Action::NoteOff(k));
                    }
                }
            }
            false
        });
    });
}

pub fn draw(root: &mut Ui, st: &mut UiState, cx: &mut Cx) {
    shortcuts(&root.ctx().clone(), st, cx);
    // Finish an edit when the pointer is released: one drag, one undo step.
    if !root.ctx().input(|i| i.pointer.any_down()) {
        cx.session.commit();
    }
    st.export.frame_sync(cx.session);

    let chrome = st.panels && !cx.presentation;
    if chrome {
        egui::Panel::top("menu").show(root, |ui| menu(ui, st, cx));
        egui::Panel::bottom("transport")
            .exact_size(92.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x12, 0x12, 0x14))
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(root, |ui| transport_bar(ui, cx));
        egui::Panel::left("tracks").resizable(true).default_size(230.0).min_size(180.0).show(
            root,
            |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| tracks_panel(ui, cx));
            },
        );
        egui::Panel::right("design").resizable(true).default_size(320.0).min_size(260.0).show(
            root,
            |ui| {
                inspector::panel(ui, &mut st.search, cx);
            },
        );
    }
    egui::CentralPanel::no_frame().show(root, |ui| {
        preview(ui, st, cx);
    });

    export::dialog(root.ctx(), st, cx);
    export::progress(root.ctx(), &mut st.thumb, cx);
}

fn menu(ui: &mut Ui, st: &mut UiState, cx: &mut Cx) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("Open MIDI…    Ctrl+O").clicked() {
                cx.actions.push(Action::OpenMidi);
                ui.close();
            }
            if ui
                .button("Save MIDI…")
                .on_hover_text("Save the loaded piece — a live recording, say — as a .mid")
                .clicked()
            {
                cx.actions.push(Action::SaveMidi);
                ui.close();
            }
            ui.separator();
            if ui.button("Open Design…").clicked() {
                cx.actions.push(Action::OpenDesign);
                ui.close();
            }
            if ui.button("Save Design As…").clicked() {
                cx.actions.push(Action::SaveDesign);
                ui.close();
            }
            ui.separator();
            if ui.button("Load SoundFont…").clicked() {
                cx.actions.push(Action::OpenSoundFont);
                ui.close();
            }
            if ui.button("Use Built-in Piano").clicked() {
                cx.actions.push(Action::BuiltinPiano);
                ui.close();
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                cx.actions.push(Action::Quit);
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.add_enabled(cx.session.can_undo(), egui::Button::new("Undo    Ctrl+Z")).clicked()
            {
                cx.session.undo();
                ui.close();
            }
            if ui
                .add_enabled(cx.session.can_redo(), egui::Button::new("Redo    Ctrl+Shift+Z"))
                .clicked()
            {
                cx.session.redo();
                ui.close();
            }
        });
        ui.menu_button("View", |ui| {
            ui.checkbox(&mut st.panels, "Panels    Tab");
            ui.checkbox(&mut st.hud, "Performance HUD    F3");
            if ui.button("Presentation    F11").clicked() {
                cx.actions.push(Action::Presentation);
                ui.close();
            }
        });
        ui.menu_button("Render", |ui| {
            if ui.button("Export Video…    Ctrl+R").clicked() {
                st.export_open = true;
                ui.close();
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let export = ui.add(
                egui::Button::new(egui::RichText::new("Export").color(Color32::BLACK)).fill(ACCENT),
            );
            if export.clicked() {
                st.export_open = true;
            }
            ui.label(egui::RichText::new(&cx.session.score_name).color(MUTED_TEXT));
        });
    });
}

fn clock(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

fn transport_bar(ui: &mut Ui, cx: &mut Cx) {
    let end = cx.session.end();
    let t = cx.time;
    ui.horizontal(|ui| {
        let playing = cx.transport.is_playing();
        if ui.add_sized([34.0, 26.0], egui::Button::new(if playing { "⏸" } else { "▶" })).on_hover_text("Play / pause (Space)").clicked() {
            cx.transport.toggle();
        }
        if ui.add_sized([30.0, 26.0], egui::Button::new("⏮")).on_hover_text("To start (Home)").clicked() {
            cx.transport.seek(0.0);
        }
        ui.label(egui::RichText::new(format!("{}  /  {}", clock(t), clock(cx.session.score.duration))).monospace().size(14.0));
        ui.add_space(12.0);
        let mut looping = cx.session.looping;
        if ui.toggle_value(&mut looping, "Loop").on_hover_text("Loop the marked region (Ctrl+L). Mark it with [ and ], or shift-drag the timeline.").changed() {
            cx.session.looping = looping;
            if looping && cx.session.loop_range.is_none() {
                cx.session.loop_range = Some((t, (t + 8.0).min(cx.session.score.duration)));
            }
        }
        ui.add_space(8.0);
        let mut speed = cx.transport.speed();
        egui::ComboBox::from_id_salt("speed").width(64.0).selected_text(format!("{speed}×")).show_ui(ui, |ui| {
            for s in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                ui.selectable_value(&mut speed, s, format!("{s}×"));
            }
        });
        if speed != cx.transport.speed() {
            cx.transport.set_speed(speed);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut vol = cx.transport.volume();
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.5).show_value(false)).on_hover_text("Volume").changed() {
                cx.transport.set_volume(vol);
            }
            ui.label(egui::RichText::new("Volume").color(MUTED_TEXT));
            ui.add_space(12.0);
            let status = &cx.transport.audio_status;
            let (short, detail) = match status.strip_prefix("no sound: ") {
                Some(why) => ("No sound device — playing silently", Some(why)),
                None => (status.as_str(), None),
            };
            let l = ui.label(egui::RichText::new(short).color(MUTED_TEXT).small());
            if let Some(d) = detail {
                l.on_hover_text(d);
            }
        });
    });
    ui.add_space(4.0);
    timeline(ui, cx, end);
}

/// Note density over the whole piece, bar lines, the loop, and the
/// playhead. Drag to scrub; shift-drag to mark a loop.
fn timeline(ui: &mut Ui, cx: &mut Cx, end: f64) {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_rgb(0x0c, 0x0c, 0x0e));
    let x_of = |t: f64| rect.left() + (t / end.max(1e-3)) as f32 * rect.width();
    let t_of = |x: f32| (((x - rect.left()) / rect.width()) as f64 * end).clamp(0.0, end);

    let dur = cx.session.score.duration.max(1e-3);
    let n = cx.session.density.len().max(1);
    let bw = (x_of(dur) - rect.left()) / n as f32;
    for (i, &d) in cx.session.density.iter().enumerate() {
        if d <= 0.0 {
            continue;
        }
        let x = rect.left() + i as f32 * bw;
        let h = (rect.height() - 8.0) * d;
        p.rect_filled(
            Rect::from_min_max(
                pos2(x, rect.bottom() - 4.0 - h),
                pos2(x + bw.max(1.0), rect.bottom() - 4.0),
            ),
            0.0,
            Color32::from_rgb(0x4a, 0x42, 0x3a),
        );
    }
    // Bar lines, thinned out so they never turn into a solid fill.
    let bars = &cx.session.bars;
    let step = (bars.len() as f32 * 6.0 / rect.width()).ceil().max(1.0) as usize;
    for &b in bars.iter().step_by(step) {
        let x = x_of(b);
        p.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.top() + 5.0)],
            Stroke::new(1.0, Color32::from_gray(60)),
        );
    }
    if let Some((a, b)) = cx.session.loop_range {
        let r = Rect::from_min_max(pos2(x_of(a), rect.top()), pos2(x_of(b), rect.bottom()));
        let alpha = if cx.session.looping { 0.22 } else { 0.08 };
        p.rect_filled(r, 2.0, ACCENT.gamma_multiply(alpha));
        p.rect_stroke(
            r,
            2.0,
            Stroke::new(1.0, ACCENT.gamma_multiply(alpha * 3.0)),
            egui::StrokeKind::Inside,
        );
    }
    let px = x_of(cx.time);
    p.line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], Stroke::new(2.0, ACCENT));

    let shift = ui.input(|i| i.modifiers.shift);
    if let Some(pos) = resp.interact_pointer_pos() {
        if shift {
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let (a, b) = (t_of(origin.x), t_of(pos.x));
            if (a - b).abs() > 0.2 {
                cx.session.loop_range = Some((a.min(b), a.max(b)));
                cx.session.looping = true;
            }
        } else if resp.dragged() || resp.clicked() {
            cx.actions.push(Action::Seek(t_of(pos.x)));
        }
    }
    if let Some(h) = resp.hover_pos() {
        resp.on_hover_text_at_pointer(clock(t_of(h.x)));
    }
}

fn tracks_panel(ui: &mut Ui, cx: &mut Cx) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new("TRACKS").small().strong().color(MUTED_TEXT));
    ui.add_space(2.0);
    let s = &mut *cx.session;
    // In live mode the loaded piece isn't what's on screen.
    let rows: Vec<(usize, String, u32)> = if cx.live.active {
        ui.label(egui::RichText::new("Showing live input").color(MUTED_TEXT));
        Vec::new()
    } else {
        s.score.visible_tracks().map(|(i, t)| (i, t.label(i), t.note_count)).collect()
    };
    if rows.is_empty() && !cx.live.active {
        ui.label(egui::RichText::new("No notes in this file").color(MUTED_TEXT));
    }
    for (n, (i, label, count)) in rows.into_iter().enumerate() {
        ui.horizontal(|ui| {
            let mut c = cx.renderer.track_color(i).0;
            if ui
                .color_edit_button_srgba_unmultiplied(&mut c)
                .on_hover_text("Track color (right-click the name to reset)")
                .changed()
            {
                s.track_colors[i] = Some(pv_design::Color(c));
                s.tracks_dirty = true;
            }
            let shown = s.visible[i];
            if ui
                .selectable_label(shown, egui::RichText::new("notes").small())
                .on_hover_text("Show this track's notes")
                .clicked()
            {
                s.visible[i] = !shown;
                s.tracks_dirty = true;
            }
            let audible = !s.muted[i];
            if ui
                .selectable_label(audible, egui::RichText::new("sound").small())
                .on_hover_text("Play this track's sound")
                .clicked()
            {
                s.muted[i] = audible;
                s.tracks_dirty = true;
            }
            let soloed = s.solo == Some(i);
            let name = egui::RichText::new(&label).color(if soloed {
                ACCENT
            } else {
                ui.visuals().text_color()
            });
            let r = ui
                .add(egui::Label::new(name).sense(Sense::click()).truncate())
                .on_hover_text(format!("{count} notes · click to solo ({})", n + 1));
            if r.clicked() {
                s.solo = if soloed { None } else { Some(i) };
                s.tracks_dirty = true;
            }
            r.context_menu(|ui| {
                if ui.button("Reset color").clicked() {
                    s.track_colors[i] = None;
                    s.tracks_dirty = true;
                    ui.close();
                }
            });
        });
    }
    if s.solo.is_some() && ui.small_button("Clear solo").clicked() {
        s.solo = None;
        s.tracks_dirty = true;
    }

    ui.add_space(14.0);
    live_panel(ui, cx);

    let s = &mut *cx.session;
    ui.add_space(14.0);
    ui.label(egui::RichText::new("DESIGN").small().strong().color(MUTED_TEXT));
    let current = match &s.design_source {
        DesignSource::Builtin(id) => pv_design::builtin(id).meta.name,
        DesignSource::File(p) => {
            p.file_name().map_or("file".into(), |n| n.to_string_lossy().into_owned())
        }
        DesignSource::Unsaved => format!("{} (edited)", s.design.meta.name),
    };
    egui::ComboBox::from_id_salt("design-pick")
        .width(ui.available_width() - 8.0)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (id, src) in pv_design::BUILTIN {
                let name = pv_design::from_toml(src)
                    .map(|l| l.design.meta.name)
                    .unwrap_or_else(|_| id.to_string());
                if ui.selectable_label(s.design_source == DesignSource::Builtin(id), name).clicked()
                {
                    cx.actions.push(Action::Builtin(id));
                }
            }
        });
    ui.horizontal(|ui| {
        if ui.button("Open…").clicked() {
            cx.actions.push(Action::OpenDesign);
        }
        if ui.button("Save as…").clicked() {
            cx.actions.push(Action::SaveDesign);
        }
    });
    if let DesignSource::File(p) = &s.design_source {
        ui.label(
            egui::RichText::new(
                "Watching for edits — change the file in any editor and the preview follows.",
            )
            .small()
            .color(MUTED_TEXT),
        )
        .on_hover_text(p.display().to_string());
    }

    ui.add_space(14.0);
    ui.label(egui::RichText::new("SOUND").small().strong().color(MUTED_TEXT));
    let mut opts = cx.transport.options();
    if ui
        .checkbox(&mut opts.force_piano, "Play everything on piano")
        .on_hover_text("Ignore program changes in the file")
        .changed()
    {
        cx.transport.set_options(opts, &s.score);
    }
    ui.horizontal(|ui| {
        if ui
            .button("SoundFont…")
            .on_hover_text("Load any .sf2 — also works by dropping it on the window")
            .clicked()
        {
            cx.actions.push(Action::OpenSoundFont);
        }
        if ui.button("Built-in").clicked() {
            cx.actions.push(Action::BuiltinPiano);
        }
    });
}

fn live_panel(ui: &mut Ui, cx: &mut Cx) {
    let live = &mut *cx.live;
    ui.label(egui::RichText::new("LIVE").small().strong().color(MUTED_TEXT));
    ui.horizontal(|ui| {
        let label = if live.active { "Live: on" } else { "Live: off" };
        let b = egui::Button::new(egui::RichText::new(label).color(if live.active {
            Color32::BLACK
        } else {
            ui.visuals().text_color()
        }));
        let b = if live.active { b.fill(ACCENT) } else { b };
        if ui.add(b).on_hover_text("Play and the visuals follow (Esc to leave)").clicked() {
            cx.actions.push(Action::Live(!live.active));
        }
        if live.active {
            let rec = live.rec.recording.is_some();
            let text = if rec {
                let secs = live.rec.now() - live.rec.recording.unwrap_or(0.0);
                format!("Stop {}:{:02}", (secs / 60.0) as u32, secs as u32 % 60)
            } else {
                "Record".into()
            };
            let b = egui::Button::new(egui::RichText::new(text).color(if rec {
                Color32::WHITE
            } else {
                ui.visuals().text_color()
            }));
            let b = if rec { b.fill(Color32::from_rgb(0xb8, 0x2e, 0x2e)) } else { b };
            if ui
                .add(b)
                .on_hover_text("Record what you play; it becomes the loaded piece")
                .clicked()
            {
                cx.actions.push(Action::Record(!rec));
            }
        }
    });
    ui.horizontal(|ui| {
        let current = live.port.clone().unwrap_or_else(|| "MIDI device…".into());
        let mut pick = None;
        egui::ComboBox::from_id_salt("midi-port")
            .width(ui.available_width() - 40.0)
            .selected_text(current)
            .show_ui(ui, |ui| {
                if live.ports.is_empty() {
                    ui.label(egui::RichText::new("No devices found").color(MUTED_TEXT));
                }
                for p in &live.ports {
                    if ui.selectable_label(live.port.as_deref() == Some(p), p).clicked() {
                        pick = Some(p.clone());
                    }
                }
            });
        if ui.small_button("⟳").on_hover_text("Look for devices again").clicked() {
            live.refresh_ports();
        }
        if let Some(p) = pick {
            cx.actions.push(Action::LiveConnect(p));
        }
    });
    if live.connected() && ui.small_button("Disconnect").clicked() {
        cx.actions.push(Action::LiveDisconnect);
    }
    ui.label(egui::RichText::new(&live.status).small().color(MUTED_TEXT));
    if live.active {
        ui.label(
            egui::RichText::new(format!(
                "Keyboard plays from C{}: A W S E D F T G… · Z/X octave · Space pedal · or click the keys",
                live.octave as i32 / 12 - 1
            ))
            .small()
            .color(MUTED_TEXT),
        );
    }
    let mut low = cx.transport.low_latency();
    if ui.checkbox(&mut low, "Low latency").on_hover_text("256-sample audio buffer: about 5 ms instead of 11, with a higher risk of clicks on a busy machine").changed() {
        cx.actions.push(Action::LowLatency(low));
    }
}

/// Which key of the preview's keyboard is under `p`, if any. Black keys
/// are tested first: they sit on top.
fn key_at(p: Pos2, rect: Rect, cx: &Cx) -> Option<u8> {
    let d = cx.renderer.design();
    let kb_h = d.keyboard.height;
    let kb_y0 = if d.background.reflection.enabled { kb_h * 0.6 } else { 0.0 };
    let y = (rect.bottom() - p.y) / rect.height();
    let x = (p.x - rect.left()) / rect.width();
    if !(kb_y0..=kb_y0 + kb_h).contains(&y) {
        return None;
    }
    let keys = &cx.renderer.key_layout().keys;
    let in_black_zone = y > kb_y0 + kb_h * (1.0 - d.keyboard.black_length_ratio);
    let hit = |black: bool| {
        keys.iter()
            .position(|k| k.visible && k.black == black && (k.x0..=k.x1).contains(&x))
            .map(|i| i as u8)
    };
    if in_black_zone && let Some(k) = hit(true) {
        return Some(k);
    }
    hit(false)
}

fn preview(ui: &mut Ui, st: &mut UiState, cx: &mut Cx) {
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 0.0, Color32::from_rgb(0x08, 0x08, 0x09));
    // Letterbox to the export frame's shape: what's on screen is framed
    // exactly as the video will be.
    let (fw, fh) = cx.session.frame_size;
    let aspect = fw as f32 / fh.max(1) as f32;
    let margin = if cx.presentation { 0.0 } else { 12.0 };
    let avail = full.shrink(margin);
    let size = if avail.width() / avail.height().max(1.0) > aspect {
        vec2(avail.height() * aspect, avail.height())
    } else {
        vec2(avail.width(), avail.width() / aspect)
    };
    let rect = Rect::from_center_size(avail.center(), size);
    let ppp = ui.ctx().pixels_per_point();
    *cx.preview_px =
        ((size.x * ppp).round().max(1.0) as u32, (size.y * ppp).round().max(1.0) as u32);

    let resp = ui.allocate_rect(full, Sense::click_and_drag());
    if let Some(id) = cx.preview_tex {
        ui.painter().image(
            id,
            rect,
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if cx.live.active {
        // Play the keyboard with the mouse: press, slide across keys, release.
        let down = ui.input(|i| i.pointer.primary_down());
        let key =
            if down { resp.interact_pointer_pos().and_then(|p| key_at(p, rect, cx)) } else { None };
        if key != st.mouse_key {
            if let Some(k) = st.mouse_key.take() {
                cx.actions.push(Action::NoteOff(k));
            }
            if let Some(k) = key {
                cx.actions.push(Action::NoteOn(k));
            }
            st.mouse_key = key;
        }
    } else if resp.clicked() && rect.contains(resp.interact_pointer_pos().unwrap_or_default()) {
        cx.transport.toggle();
        st.hint_dismissed = true;
    }
    if resp.double_clicked() {
        cx.actions.push(Action::Presentation);
    }

    let p = ui.painter_at(full);
    if !st.hint_dismissed && !cx.presentation {
        let text = "Drop a MIDI file anywhere to begin";
        let pos = pos2(rect.center().x, rect.top() + 28.0);
        let galley =
            p.layout_no_wrap(text.into(), FontId::proportional(14.0), Color32::from_gray(220));
        let bg = Rect::from_center_size(pos, galley.size() + vec2(24.0, 12.0));
        p.rect_filled(bg, 14.0, Color32::from_black_alpha(170));
        p.galley(bg.min + vec2(12.0, 6.0), galley, Color32::WHITE);
    }

    // Messages, newest at the bottom.
    let mut y = rect.bottom() - 12.0;
    for m in cx.session.messages.iter().rev().take(4) {
        let color =
            if m.error { Color32::from_rgb(0xff, 0x9a, 0x8a) } else { Color32::from_gray(225) };
        let galley =
            p.layout(m.text.clone(), FontId::proportional(13.0), color, rect.width() - 60.0);
        let r = Rect::from_min_size(
            pos2(rect.left() + 14.0, y - galley.size().y - 10.0),
            galley.size() + vec2(20.0, 10.0),
        );
        p.rect_filled(r, 6.0, Color32::from_black_alpha(190));
        if m.error {
            p.rect_stroke(
                r,
                6.0,
                Stroke::new(1.0, Color32::from_rgb(0xc0, 0x50, 0x40)),
                egui::StrokeKind::Inside,
            );
        }
        p.galley(r.min + vec2(10.0, 5.0), galley, color);
        y = r.top() - 6.0;
    }

    if st.hud {
        hud(&p, rect, cx);
    }
}

fn hud(p: &egui::Painter, rect: Rect, cx: &Cx) {
    let n = cx.frame_times.len().max(1) as f32;
    let avg = cx.frame_times.iter().sum::<f32>() / n;
    let worst = cx.frame_times.iter().fold(0f32, |m, &v| m.max(v));
    let s = cx.renderer.stats();
    let lat = cx.transport.latency().map_or("—".into(), |l| format!("{:.1} ms", l * 1000.0));
    let text = format!(
        "{:.0} fps   avg {:.1} ms   worst {:.1} ms\nnotes {}   particle notes {}   instances {}\naudio latency {lat}\n{}",
        1.0 / avg.max(1e-4),
        avg * 1000.0,
        worst * 1000.0,
        s.visible_notes,
        s.particle_notes,
        s.particle_instances,
        cx.gpu_name
    );
    let galley = p.layout_no_wrap(text, FontId::monospace(11.5), Color32::from_gray(230));
    let r = Rect::from_min_size(rect.min + vec2(10.0, 10.0), galley.size() + vec2(16.0, 12.0));
    p.rect_filled(r, 4.0, Color32::from_black_alpha(180));
    p.galley(r.min + vec2(8.0, 6.0), galley, Color32::WHITE);
    let _ = Align2::LEFT_TOP;
}
