//! piano-viz: the desktop app. See docs/10-ui-ux.md.

mod exporter;
mod session;
mod transport;
mod ui;

use anyhow::{Context, Result};
use pv_core::Score;
use pv_render::{FrameParams, Gpu, Renderer, wgpu};
use session::{DesignSource, Session};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use transport::Transport;
use ui::{Action, UiState};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

/// Format of the preview texture: plain RGBA8. The composite shader writes
/// sRGB-encoded values itself, which is also what egui expects to sample.
const PREVIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() -> Result<()> {
    let event_loop = EventLoop::new().context("could not open a window system connection")?;
    let mut app =
        App { state: None, open: std::env::args_os().nth(1).map(PathBuf::from), failed: None };
    event_loop.run_app(&mut app)?;
    if let Some(e) = app.failed {
        return Err(e);
    }
    Ok(())
}

struct App {
    state: Option<State>,
    /// A file passed on the command line.
    open: Option<PathBuf>,
    failed: Option<anyhow::Error>,
}

/// The preview: the renderer drawing into a texture that egui shows,
/// letterboxed to the export frame's shape.
struct Preview {
    renderer: Renderer,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    size: (u32, u32),
    tex_id: Option<egui::TextureId>,
}

pub struct State {
    window: Arc<Window>,
    gpu: Arc<Gpu>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    preview: Preview,
    session: Session,
    transport: Transport,
    ui: UiState,
    export: Option<exporter::ExportRun>,
    frame_times: Vec<f32>,
    last_frame: Instant,
    presentation: bool,
    quit: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.open.take()) {
            Ok(s) => self.state = Some(s),
            Err(e) => {
                self.failed = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        let consumed = s.egui_state.on_window_event(&s.window, &event).consumed;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                s.config.width = size.width.max(1);
                s.config.height = size.height.max(1);
                s.surface.configure(&s.gpu.device, &s.config);
            }
            WindowEvent::DroppedFile(path) => s.open_path(&path),
            WindowEvent::RedrawRequested => {
                if let Err(e) = s.redraw() {
                    eprintln!("frame error: {e:#}");
                }
                if s.quit {
                    event_loop.exit();
                }
                // Don't request another frame from here: about_to_wait
                // decides, so a paused app goes idle.
                return;
            }
            _ => {}
        }
        let _ = consumed;
        s.window.request_redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(s) = &mut self.state else { return };
        // Redraw continuously only while something moves. Otherwise sleep
        // until input arrives or egui asked for a repaint at a set time
        // (an animation, a tooltip, a message expiring).
        let busy = s.transport.is_playing() || s.export.is_some() || !s.transport.audio_ready();
        if busy || s.ui.repaint_now {
            s.window.request_redraw();
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }
        let mut due = s.ui.repaint_at;
        if !s.session.messages.is_empty() {
            // Messages expire on a timer egui knows nothing about.
            let soon = Instant::now() + Duration::from_millis(500);
            due = Some(due.map_or(soon, |d| d.min(soon)));
        }
        match due {
            Some(at) if Instant::now() >= at => {
                s.ui.repaint_at = None;
                s.window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Poll);
            }
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl State {
    fn new(event_loop: &ActiveEventLoop, open: Option<PathBuf>) -> Result<Self> {
        let attrs = Window::default_attributes()
            .with_title("piano-viz")
            .with_inner_size(winit::dpi::LogicalSize::new(1440.0, 860.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(720.0, 480.0));
        let window = Arc::new(event_loop.create_window(attrs)?);

        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance.create_surface(window.clone())?;
        let gpu = Arc::new(pollster::block_on(Gpu::with_instance(instance, Some(&surface)))?);
        eprintln!("GPU: {}", gpu.describe());

        let caps = surface.get_capabilities(&gpu.adapter);
        // egui wants a non-sRGB swapchain; it does its own gamma.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            // Plain SDR: the preview is already display-encoded.
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);

        let egui_ctx = egui::Context::default();
        ui::style(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.max_texture_size() as usize),
        );
        let egui_renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());

        // First run: the demo, already on its way to playing. Never an empty
        // grey window with a "File → Open" hint.
        let demo = pv_midi::parse(&pv_midi::demo::prelude()).expect("demo parses");
        let session = Session::new(demo, "Demo — Bach, Prelude in C");
        let mut transport = Transport::new();
        transport.play_when_ready();

        let renderer = Renderer::new(&gpu, PREVIEW_FORMAT, 16, 16, 1);
        let mut s = Self {
            window,
            gpu,
            surface,
            config,
            egui_ctx,
            egui_state,
            egui_renderer,
            preview: Preview { renderer, texture: None, size: (0, 0), tex_id: None },
            session,
            transport,
            ui: UiState::default(),
            export: None,
            frame_times: Vec::with_capacity(120),
            last_frame: Instant::now(),
            presentation: false,
            quit: false,
        };
        s.preview.renderer.set_score(&s.gpu, s.session.score.clone());
        if let Some(p) = open {
            s.open_path(&p);
        }
        Ok(s)
    }

    /// Open whatever was dropped or passed in: MIDI, a Design, a SoundFont.
    fn open_path(&mut self, path: &Path) {
        let ext =
            path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "mid" | "midi" | "smf" | "kar" => match pv_midi::load(path) {
                Ok(score) => self.load_score(score, path),
                Err(e) => self.session.error(e.to_string()),
            },
            "toml" => self.session.load_design_file(path),
            "sf2" => match pv_audio::load_font(path) {
                Ok(font) => {
                    self.transport.set_font(font, &self.session.score);
                    self.session.info(format!("SoundFont: {}", path.display()));
                }
                Err(e) => self.session.error(e.to_string()),
            },
            _ if path.is_dir() && path.join("design.toml").is_file() => {
                self.session.load_design_file(path)
            }
            _ => self.session.error(format!(
                "Can't open {}: expected a .mid, a Design, or a .sf2",
                path.display()
            )),
        }
        self.ui.hint_dismissed = true;
    }

    fn load_score(&mut self, score: Score, path: &Path) {
        let name = score.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
            path.file_stem().map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
        });
        self.session.set_score(score, &name);
        self.preview.renderer.set_score(&self.gpu, self.session.score.clone());
        self.transport.set_score(&self.session.score, &self.session.silenced());
        self.ui.midi_path = Some(path.to_path_buf());
        self.window.set_title(&format!("{name} — piano-viz"));
    }

    fn toggle_presentation(&mut self) {
        self.presentation = !self.presentation;
        self.window.set_fullscreen(self.presentation.then_some(Fullscreen::Borderless(None)));
    }

    fn redraw(&mut self) -> Result<()> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if self.frame_times.len() == 120 {
            self.frame_times.remove(0);
        }
        self.frame_times.push(dt);

        self.transport.update(&self.session.score);
        self.session.poll_reload();
        self.session.expire_messages();
        if let Some(run) = &mut self.export {
            run.poll();
        }
        self.apply_changes();

        // Looping, and stopping after the end.
        let mut t = self.transport.now();
        if self.transport.is_playing() {
            if let (true, Some((a, b))) = (self.session.looping, self.session.loop_range)
                && t >= b
            {
                self.transport.seek(a);
                t = a;
            } else if t >= self.session.end() {
                self.transport.pause();
                self.transport.seek(self.session.end());
                t = self.session.end();
            }
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                return Ok(());
            }
            // Minimized or momentarily unavailable: skip this frame.
            _ => return Ok(()),
        };
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());

        // The preview renders first, into its texture; egui then shows it.
        // Paused during export, which has the GPU to itself.
        if self.export.is_none()
            && let Some((_, view)) = &self.preview.texture
        {
            self.preview.renderer.render(
                &self.gpu,
                &mut encoder,
                view,
                FrameParams { time: t, frame: 0, alpha: false },
            );
        }

        let input = self.egui_state.take_egui_input(&self.window);
        let mut actions = Vec::new();
        let mut preview_px = self.preview.size;
        let full = self.egui_ctx.clone().run_ui(input, |root| {
            let mut cx = ui::Cx {
                session: &mut self.session,
                transport: &mut self.transport,
                renderer: &self.preview.renderer,
                export: &mut self.export,
                preview_tex: self.preview.tex_id,
                preview_px: &mut preview_px,
                frame_times: &self.frame_times,
                gpu_name: self.gpu.describe(),
                presentation: self.presentation,
                actions: &mut actions,
                time: t,
            };
            ui::draw(root, &mut self.ui, &mut cx);
        });
        self.egui_state.handle_platform_output(&self.window, full.platform_output);
        if let Some(v) = full.viewport_output.get(&egui::ViewportId::ROOT) {
            self.ui.repaint_now = v.repaint_delay.is_zero();
            self.ui.repaint_at =
                (v.repaint_delay < Duration::from_secs(3600)).then(|| now + v.repaint_delay);
        }

        let ppp = full.pixels_per_point;
        let jobs = self.egui_ctx.tessellate(full.shapes, ppp);
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                self.egui_renderer.update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ppp,
        };
        let extra = self.egui_renderer.update_buffers(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let view = frame.texture.create_view(&Default::default());
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.025,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.egui_renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        self.gpu.queue.submit(extra.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        for id in &full.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        // The preview's size follows its panel; resize for next frame.
        if preview_px != self.preview.size {
            self.resize_preview(preview_px);
        }
        if let Some(run) = &mut self.export
            && run.result.is_some()
        {
            match run.result.take().unwrap() {
                Ok(s) => self.session.info(format!(
                    "Exported {} ({:.0} s of video in {:.0} s)",
                    s.output.display(),
                    s.seconds,
                    s.elapsed.as_secs_f64()
                )),
                Err(e) if e == "export cancelled" => self.session.info("Export cancelled".into()),
                Err(e) => self.session.error(format!("Export failed: {e}")),
            }
            self.export = None;
            // This frame still showed the progress window; draw one more.
            self.ui.repaint_now = true;
        }
        for a in actions {
            self.handle(a);
        }
        Ok(())
    }

    /// Actions from the UI, handled after the frame is drawn.
    fn handle(&mut self, action: Action) {
        match action {
            Action::OpenMidi => {
                if let Some(p) =
                    rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi", "kar"]).pick_file()
                {
                    self.open_path(&p);
                }
            }
            Action::OpenDesign => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Design", &["toml"]).pick_file()
                {
                    self.session.load_design_file(&p);
                }
            }
            Action::SaveDesign => {
                let name = format!(
                    "{}.toml",
                    self.session.design.meta.name.to_lowercase().replace(' ', "-")
                );
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Design", &["toml"])
                    .set_file_name(name)
                    .save_file()
                {
                    self.session.save_design(&p);
                }
            }
            Action::OpenSoundFont => {
                if let Some(p) =
                    rfd::FileDialog::new().add_filter("SoundFont", &["sf2"]).pick_file()
                {
                    self.open_path(&p);
                }
            }
            Action::BuiltinPiano => {
                self.transport.set_font(pv_audio::builtin_font(), &self.session.score);
                self.session.info("Built-in piano".into());
            }
            Action::Builtin(id) => {
                self.session.set_design(pv_design::builtin(id), DesignSource::Builtin(id), None);
            }
            Action::Seek(t) => self.transport.seek(t),
            Action::StartExport(job) => {
                self.transport.pause();
                self.export = Some(exporter::ExportRun::start(self.gpu.clone(), *job));
            }
            Action::Presentation => self.toggle_presentation(),
            Action::ExitPresentation => {
                if self.presentation {
                    self.toggle_presentation();
                }
            }
            Action::Quit => self.quit = true,
        }
    }

    /// Push Design and track changes to the renderer and the audio.
    fn apply_changes(&mut self) {
        let s = &mut self.session;
        if s.design_dirty {
            s.design_dirty = false;
            let warnings =
                self.preview.renderer.set_design(&self.gpu, &s.design, s.bundle_dir.as_deref());
            for w in warnings {
                s.error(w);
            }
            s.tracks_dirty = true;
        }
        if s.tracks_dirty {
            s.tracks_dirty = false;
            self.preview.renderer.set_track_visibility(&self.gpu, &s.shown());
            self.preview.renderer.set_track_colors(&self.gpu, &s.track_colors);
            self.transport.set_muted(&s.silenced());
        }
    }

    fn resize_preview(&mut self, (w, h): (u32, u32)) {
        self.preview.size = (w, h);
        if w < 8 || h < 8 {
            self.preview.texture = None;
            return;
        }
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PREVIEW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        match self.preview.tex_id {
            Some(id) => self.egui_renderer.update_egui_texture_from_wgpu_texture(
                &self.gpu.device,
                &view,
                wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.preview.tex_id = Some(self.egui_renderer.register_native_texture(
                    &self.gpu.device,
                    &view,
                    wgpu::FilterMode::Linear,
                ))
            }
        }
        // Supersample the preview only when it's small enough to afford.
        let ss = if w * h <= 1280 * 720 { 2 } else { 1 };
        self.preview.renderer.resize(&self.gpu, w, h, ss);
        self.preview.texture = Some((texture, view));
    }
}
