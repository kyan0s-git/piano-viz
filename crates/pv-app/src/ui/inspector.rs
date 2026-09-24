//! The Design inspector. Its sections mirror the Design file's, so the panel
//! and the TOML teach each other. Every row resets to its default from a
//! right-click, and the search box filters across all sections.

use super::{Cx, MUTED_TEXT};
use egui::{RichText, Ui};
use pv_design::*;
use std::ops::RangeInclusive;

/// Builds labelled rows, skipping those the search excludes.
struct Rows<'a> {
    filter: &'a str,
    section_matches: bool,
    changed: bool,
    id: usize,
}

impl Rows<'_> {
    fn shows(&self, label: &str) -> bool {
        self.section_matches || self.filter.is_empty() || label.to_lowercase().contains(self.filter)
    }

    fn row<T: PartialEq + Clone>(
        &mut self,
        ui: &mut Ui,
        label: &str,
        v: &mut T,
        default: &T,
        add: impl FnOnce(&mut Ui, &mut T) -> egui::Response,
    ) {
        if !self.shows(label) {
            return;
        }
        self.id += 1;
        let l = ui.add(egui::Label::new(label).sense(egui::Sense::click()).truncate());
        l.context_menu(|ui| {
            if ui.add_enabled(v != default, egui::Button::new("Reset to default")).clicked() {
                *v = default.clone();
                self.changed = true;
                ui.close();
            }
        });
        if add(ui, v).changed() {
            self.changed = true;
        }
        ui.end_row();
    }

    fn float(
        &mut self,
        ui: &mut Ui,
        label: &str,
        v: &mut f32,
        default: f32,
        range: RangeInclusive<f32>,
    ) {
        self.row(ui, label, v, &default, |ui, v| {
            ui.add(
                egui::Slider::new(v, range).clamping(egui::SliderClamping::Never).max_decimals(2),
            )
        });
    }

    fn count(
        &mut self,
        ui: &mut Ui,
        label: &str,
        v: &mut u32,
        default: u32,
        range: RangeInclusive<u32>,
    ) {
        self.row(ui, label, v, &default, |ui, v| ui.add(egui::Slider::new(v, range)));
    }

    fn toggle(&mut self, ui: &mut Ui, label: &str, v: &mut bool, default: bool) {
        self.row(ui, label, v, &default, |ui, v| ui.checkbox(v, ""));
    }

    fn color(&mut self, ui: &mut Ui, label: &str, v: &mut Color, default: Color) {
        self.row(ui, label, v, &default, |ui, v| ui.color_edit_button_srgba_unmultiplied(&mut v.0));
    }

    fn choice<T: PartialEq + Copy>(
        &mut self,
        ui: &mut Ui,
        label: &str,
        v: &mut T,
        default: T,
        options: &[(T, &str)],
    ) {
        let id = self.id;
        self.row(ui, label, v, &default, |ui, v| {
            let current = options.iter().find(|(o, _)| o == v).map_or("?", |(_, n)| n);
            let mut changed = false;
            let mut r = egui::ComboBox::from_id_salt(("choice", label, id))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (o, name) in options {
                        changed |= ui.selectable_value(v, *o, *name).changed();
                    }
                })
                .response;
            if changed {
                r.mark_changed();
            }
            r
        });
    }
}

fn section(
    ui: &mut Ui,
    title: &str,
    filter: &str,
    open: bool,
    changed: &mut bool,
    body: impl FnOnce(&mut Ui, &mut Rows),
) {
    let section_matches = !filter.is_empty() && title.to_lowercase().contains(filter);
    let mut header = egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(open);
    if !filter.is_empty() {
        header = header.open(Some(true));
    }
    header.show(ui, |ui| {
        egui::Grid::new(("grid", title))
            .num_columns(2)
            .spacing([10.0, 6.0])
            .min_col_width(96.0)
            .show(ui, |ui| {
                let mut rows = Rows { filter, section_matches, changed: false, id: 0 };
                body(ui, &mut rows);
                *changed |= rows.changed;
            });
    });
}

pub fn panel(ui: &mut Ui, search: &mut String, cx: &mut Cx) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("LOOK").small().strong().color(MUTED_TEXT));
        ui.add(
            egui::TextEdit::singleline(search)
                .hint_text("Search settings")
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(4.0);
    let filter = search.trim().to_lowercase();
    let f = filter.as_str();
    let def = Design::default();
    let d = &mut cx.session.design;
    let mut changed = false;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        section(ui, "Notes", f, true, &mut changed, |ui, r| {
            let (n, dn) = (&mut d.notes, &def.notes);
            r.choice(
                ui,
                "Color by",
                &mut n.color.source,
                dn.color.source,
                &[
                    (ColorSource::Track, "Track"),
                    (ColorSource::Hand, "Hand"),
                    (ColorSource::PitchClass, "Pitch class"),
                    (ColorSource::Velocity, "Velocity"),
                    (ColorSource::Channel, "Channel"),
                    (ColorSource::Fixed, "Single color"),
                ],
            );
            if r.shows("palette") {
                ui.label("Palette");
                ui.horizontal_wrapped(|ui| {
                    let mut remove = None;
                    for (i, c) in n.color.palette.iter_mut().enumerate() {
                        let resp = ui.color_edit_button_srgba_unmultiplied(&mut c.0);
                        r.changed |= resp.changed();
                        resp.context_menu(|ui| {
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                                ui.close();
                            }
                        });
                    }
                    if let Some(i) = remove.filter(|_| n.color.palette.len() > 1) {
                        n.color.palette.remove(i);
                        r.changed = true;
                    }
                    if n.color.palette.len() < 16
                        && ui.small_button("+").on_hover_text("Add a color").clicked()
                    {
                        let last = *n.color.palette.last().unwrap_or(&Color::WHITE);
                        n.color.palette.push(last);
                        r.changed = true;
                    }
                });
                ui.end_row();
            }
            r.float(ui, "Brightness", &mut n.intensity, dn.intensity, 0.0..=6.0);
            r.float(
                ui,
                "Velocity brightness",
                &mut n.color.velocity_brightness,
                dn.color.velocity_brightness,
                0.0..=1.0,
            );
            r.float(ui, "Active boost", &mut n.active_boost, dn.active_boost, 1.0..=4.0);
            r.float(ui, "Opacity", &mut n.opacity, dn.opacity, 0.0..=1.0);
            r.float(ui, "Corner radius", &mut n.corner_radius, dn.corner_radius, 0.0..=30.0);
            r.float(ui, "Border width", &mut n.border_width, dn.border_width, 0.0..=8.0);
            r.color(ui, "Border color", &mut n.border_color, dn.border_color);
            r.choice(
                ui,
                "Gradient",
                &mut n.gradient,
                dn.gradient,
                &[
                    (GradientDir::None, "None"),
                    (GradientDir::Along, "Along"),
                    (GradientDir::Across, "Across"),
                ],
            );
            r.float(
                ui,
                "Gradient falloff",
                &mut n.gradient_falloff,
                dn.gradient_falloff,
                0.0..=1.0,
            );
            r.float(ui, "Gap to key edge", &mut n.margin, dn.margin, 0.0..=8.0);
            r.float(
                ui,
                "Black-key shade",
                &mut n.color.black_key_shade,
                dn.color.black_key_shade,
                0.3..=1.0,
            );
            r.toggle(ui, "Pedal tails", &mut n.sustain_tail.enabled, dn.sustain_tail.enabled);
            r.float(
                ui,
                "Pedal tail opacity",
                &mut n.sustain_tail.opacity,
                dn.sustain_tail.opacity,
                0.0..=1.0,
            );
        });

        section(ui, "Particles", f, true, &mut changed, |ui, r| {
            let mut remove = None;
            let n = d.particles.len();
            for (i, e) in d.particles.iter_mut().enumerate() {
                let de = if e.event == EmitEvent::NoteHold {
                    Emitter::hold_default()
                } else {
                    Emitter::default()
                };
                let title = match e.event {
                    EmitEvent::NoteHit => "Burst on strike",
                    EmitEvent::NoteHold => "Stream while held",
                    EmitEvent::NoteRelease => "Burst on release",
                };
                if r.shows(title) || r.shows("particle") || !r.filter.is_empty() {
                    ui.label(RichText::new(format!("{}. {title}", i + 1)).color(super::ACCENT));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
                r.toggle(ui, "Enabled", &mut e.enabled, true);
                r.choice(
                    ui,
                    "When",
                    &mut e.event,
                    de.event,
                    &[
                        (EmitEvent::NoteHit, "Note strikes"),
                        (EmitEvent::NoteHold, "While held"),
                        (EmitEvent::NoteRelease, "Note ends"),
                    ],
                );
                if e.event == EmitEvent::NoteHold {
                    r.float(ui, "Rate /s", &mut e.rate, de.rate, 0.0..=80.0);
                } else {
                    r.count(ui, "Count", &mut e.count, de.count, 0..=MAX_BURST);
                }
                r.float(ui, "Lifetime", &mut e.lifetime, de.lifetime, 0.05..=6.0);
                r.float(ui, "Speed", &mut e.speed, de.speed, 0.0..=800.0);
                r.float(ui, "Spread °", &mut e.spread_degrees, de.spread_degrees, 0.0..=360.0);
                r.float(
                    ui,
                    "Direction °",
                    &mut e.direction_degrees,
                    de.direction_degrees,
                    0.0..=360.0,
                );
                r.float(ui, "Gravity", &mut e.gravity[1], de.gravity[1], -600.0..=600.0);
                r.float(ui, "Drag", &mut e.drag, de.drag, 0.0..=8.0);
                r.float(ui, "Turbulence", &mut e.turbulence, de.turbulence, 0.0..=150.0);
                r.float(ui, "Size", &mut e.size, de.size, 0.2..=12.0);
                r.float(ui, "Brightness", &mut e.intensity, de.intensity, 0.0..=10.0);
                r.float(ui, "White-hot", &mut e.white_hot, de.white_hot, 0.0..=1.0);
                r.float(
                    ui,
                    "Velocity response",
                    &mut e.velocity_response,
                    de.velocity_response,
                    0.0..=1.0,
                );
                r.choice(
                    ui,
                    "Color",
                    &mut e.color_source,
                    de.color_source,
                    &[
                        (ParticleColor::Note, "Note's color"),
                        (ParticleColor::Palette, "Palette"),
                        (ParticleColor::Fixed, "Fixed"),
                    ],
                );
                if e.color_source == ParticleColor::Fixed {
                    r.color(ui, "Fixed color", &mut e.color, de.color);
                }
                if i + 1 < n {
                    ui.separator();
                    ui.end_row();
                }
            }
            if let Some(i) = remove {
                d.particles.remove(i);
                r.changed = true;
            }
            if d.particles.len() < MAX_EMITTERS && r.filter.is_empty() {
                ui.label("");
                ui.horizontal(|ui| {
                    if ui.small_button("+ Burst").clicked() {
                        d.particles.push(Emitter::default());
                        r.changed = true;
                    }
                    if ui.small_button("+ Stream").clicked() {
                        d.particles.push(Emitter::hold_default());
                        r.changed = true;
                    }
                });
                ui.end_row();
            }
        });

        section(ui, "Glow & color", f, true, &mut changed, |ui, r| {
            let (p, dp) = (&mut d.post, &def.post);
            r.toggle(ui, "Bloom", &mut p.bloom.enabled, dp.bloom.enabled);
            r.float(ui, "Bloom strength", &mut p.bloom.intensity, dp.bloom.intensity, 0.0..=3.0);
            r.float(ui, "Bloom width", &mut p.bloom.radius, dp.bloom.radius, 0.0..=1.0);
            r.float(ui, "Bloom threshold", &mut p.bloom.threshold, dp.bloom.threshold, 0.0..=3.0);
            r.color(ui, "Bloom tint", &mut p.bloom.tint, dp.bloom.tint);
            r.choice(
                ui,
                "Tonemap",
                &mut p.tonemap.operator,
                dp.tonemap.operator,
                &[
                    (TonemapOp::Agx, "AgX"),
                    (TonemapOp::Aces, "ACES"),
                    (TonemapOp::Reinhard, "Reinhard"),
                    (TonemapOp::None, "None"),
                ],
            );
            r.float(ui, "Exposure", &mut p.tonemap.exposure, dp.tonemap.exposure, -3.0..=3.0);
            r.float(ui, "Saturation", &mut p.tonemap.saturation, dp.tonemap.saturation, 0.0..=2.0);
            r.toggle(ui, "Vignette", &mut p.vignette.enabled, dp.vignette.enabled);
            r.float(ui, "Vignette amount", &mut p.vignette.amount, dp.vignette.amount, 0.0..=1.0);
            r.toggle(ui, "Film grain", &mut p.grain.enabled, dp.grain.enabled);
            r.float(ui, "Grain amount", &mut p.grain.amount, dp.grain.amount, 0.0..=0.15);
            r.toggle(
                ui,
                "Chromatic aberration",
                &mut p.chromatic_aberration.enabled,
                dp.chromatic_aberration.enabled,
            );
            r.float(
                ui,
                "Aberration amount",
                &mut p.chromatic_aberration.amount,
                dp.chromatic_aberration.amount,
                0.0..=0.01,
            );
        });

        section(ui, "Keyboard", f, false, &mut changed, |ui, r| {
            let (k, dk) = (&mut d.keyboard, &def.keyboard);
            r.choice(
                ui,
                "Keys",
                &mut k.range,
                dk.range,
                &[
                    (KeyRange::Fixed(21, 108), "88 keys"),
                    (KeyRange::Fixed(28, 103), "76 keys"),
                    (KeyRange::Fixed(36, 96), "61 keys"),
                    (KeyRange::Fixed(36, 84), "49 keys"),
                    (KeyRange::Auto, "Fit the music"),
                ],
            );
            r.float(ui, "Height", &mut k.height, dk.height, 0.0..=0.4);
            r.color(ui, "White keys", &mut k.white_key_color, dk.white_key_color);
            r.color(ui, "Black keys", &mut k.black_key_color, dk.black_key_color);
            r.float(
                ui,
                "Black key width",
                &mut k.black_width_ratio,
                dk.black_width_ratio,
                0.3..=0.9,
            );
            r.float(
                ui,
                "Black key length",
                &mut k.black_length_ratio,
                dk.black_length_ratio,
                0.3..=0.9,
            );
            r.toggle(
                ui,
                "Tint pressed keys",
                &mut k.pressed.tint_from_note,
                dk.pressed.tint_from_note,
            );
            r.float(
                ui,
                "Tint amount",
                &mut k.pressed.tint_amount,
                dk.pressed.tint_amount,
                0.0..=1.0,
            );
            r.float(
                ui,
                "Key glow",
                &mut k.pressed.glow_intensity,
                dk.pressed.glow_intensity,
                0.0..=6.0,
            );
            r.float(
                ui,
                "Glow height",
                &mut k.pressed.glow_radius,
                dk.pressed.glow_radius,
                0.0..=120.0,
            );
            r.float(
                ui,
                "Key travel",
                &mut k.pressed.depress_pixels,
                dk.pressed.depress_pixels,
                0.0..=8.0,
            );
            r.toggle(ui, "Strike line", &mut k.strike_line.enabled, dk.strike_line.enabled);
            r.color(ui, "Strike line color", &mut k.strike_line.color, dk.strike_line.color);
            r.float(
                ui,
                "Strike line glow",
                &mut k.strike_line.intensity,
                dk.strike_line.intensity,
                0.0..=5.0,
            );
        });

        section(ui, "Background", f, false, &mut changed, |ui, r| {
            let (b, db) = (&mut d.background, &def.background);
            r.choice(
                ui,
                "Type",
                &mut b.kind,
                db.kind,
                &[
                    (BackgroundKind::Solid, "Solid"),
                    (BackgroundKind::Gradient, "Gradient"),
                    (BackgroundKind::Image, "Image"),
                ],
            );
            match b.kind {
                BackgroundKind::Solid => r.color(ui, "Color", &mut b.color, db.color),
                BackgroundKind::Gradient => {
                    for (i, s) in b.stops.iter_mut().enumerate() {
                        let dc = db.stops.get(i).map_or(s.color, |x| x.color);
                        r.color(ui, &format!("Stop {}", i + 1), &mut s.color, dc);
                    }
                    r.float(ui, "Angle °", &mut b.angle, db.angle, 0.0..=360.0);
                    r.toggle(ui, "Radial", &mut b.radial, db.radial);
                }
                BackgroundKind::Image => {
                    if r.shows("image") {
                        ui.label("Image");
                        ui.horizontal(|ui| {
                            r.changed |= ui
                                .add(egui::TextEdit::singleline(&mut b.image).desired_width(140.0))
                                .lost_focus();
                            if ui.small_button("…").clicked()
                                && let Some(p) = rfd::FileDialog::new()
                                    .add_filter("Image", &["png", "jpg", "jpeg"])
                                    .pick_file()
                            {
                                b.image = p.display().to_string();
                                r.changed = true;
                            }
                        });
                        ui.end_row();
                    }
                    r.choice(
                        ui,
                        "Fit",
                        &mut b.fit,
                        db.fit,
                        &[
                            (ImageFit::Fill, "Fill"),
                            (ImageFit::Fit, "Fit"),
                            (ImageFit::Stretch, "Stretch"),
                        ],
                    );
                    r.color(ui, "Tint", &mut b.tint, db.tint);
                }
            }
            r.toggle(ui, "Reflection", &mut b.reflection.enabled, db.reflection.enabled);
            r.float(
                ui,
                "Reflection opacity",
                &mut b.reflection.opacity,
                db.reflection.opacity,
                0.0..=1.0,
            );
            r.float(ui, "Reflection blur", &mut b.reflection.blur, db.reflection.blur, 0.0..=16.0);
        });

        section(ui, "Motion", f, false, &mut changed, |ui, r| {
            let (c, dc) = (&mut d.camera, &def.camera);
            r.float(ui, "Lookahead (s)", &mut d.layout.lookahead, def.layout.lookahead, 0.5..=8.0);
            r.choice(
                ui,
                "Direction",
                &mut d.layout.direction,
                def.layout.direction,
                &[(Direction::Down, "Falling"), (Direction::Up, "Rising")],
            );
            r.float(ui, "Zoom", &mut c.zoom, dc.zoom, 0.5..=2.0);
            r.toggle(ui, "Impact shake", &mut c.shake.enabled, dc.shake.enabled);
            r.float(ui, "Shake amount", &mut c.shake.amount, dc.shake.amount, 0.0..=12.0);
            r.toggle(ui, "Drift", &mut c.drift.enabled, dc.drift.enabled);
            r.float(ui, "Drift amount", &mut c.drift.amount, dc.drift.amount, 0.0..=30.0);
        });
        ui.add_space(20.0);
    });
    if changed {
        cx.session.edited();
    }
}
