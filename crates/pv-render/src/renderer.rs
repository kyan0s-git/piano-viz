use crate::frame::{active_notes, camera_offset, key_states};
use crate::layout::KeyLayout;
use crate::uniforms::{self, BloomPass, Frame, Globals, KeyGpu, MAX_TRACKS, PostGpu};
use crate::{Gpu, image};
use pv_core::Score;
use pv_design::{Color, Design, Direction};
use std::path::Path;
use std::sync::Arc;
use wgpu::util::DeviceExt;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const BLOOM_LEVELS: usize = 6;

/// What changes from one frame to the next.
#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// Playhead, seconds.
    pub time: f64,
    /// Frame counter, for callers' bookkeeping. Rendering depends only on
    /// `time`, so a moment looks identical at any frame rate.
    pub frame: u64,
    /// Transparent background for compositing.
    pub alpha: bool,
}

/// Counts for the performance HUD.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub visible_notes: u32,
    pub particle_notes: u32,
    pub particle_instances: u32,
}

struct Pipelines {
    background: wgpu::RenderPipeline,
    notes_white: wgpu::RenderPipeline,
    notes_black: wgpu::RenderPipeline,
    keys_white: wgpu::RenderPipeline,
    keys_black: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    particles: wgpu::RenderPipeline,
    bloom_down: wgpu::RenderPipeline,
    bloom_up: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

struct Targets {
    scene: wgpu::TextureView,
    bloom: Vec<(wgpu::TextureView, [u32; 2])>,
    down: Vec<wgpu::BindGroup>,
    up: Vec<wgpu::BindGroup>,
    down_bufs: Vec<wgpu::Buffer>,
    up_bufs: Vec<wgpu::Buffer>,
    composite: wgpu::BindGroup,
}

/// Draws a score in a Design. One instance renders for both preview and
/// export; only the time source and the target differ.
pub struct Renderer {
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    supersample: u32,

    scene_bgl: wgpu::BindGroupLayout,
    bloom_bgl: wgpu::BindGroupLayout,
    composite_bgl: wgpu::BindGroupLayout,
    pipes: Pipelines,
    sampler: wgpu::Sampler,

    globals_buf: wgpu::Buffer,
    particles_buf: wgpu::Buffer,
    post_buf: wgpu::Buffer,
    notes_buf: wgpu::Buffer,
    tracks_buf: wgpu::Buffer,
    keys_buf: wgpu::Buffer,
    active_buf: wgpu::Buffer,
    active_capacity: u32,
    bg_view: wgpu::TextureView,
    scene_bg: wgpu::BindGroup,
    targets: Targets,

    score: Arc<Score>,
    design: Design,
    layout: KeyLayout,
    frame: Frame,
    globals: Globals,
    post: PostGpu,
    max_life: [f32; 3],
    /// Particle instances per active note.
    stride: u32,
    /// (left, right) track indices when hand coloring can use the tracks.
    hands: Option<(usize, usize)>,
    image_aspect: Option<f32>,
    track_visible: Vec<bool>,
    track_colors: Vec<Option<Color>>,
    keys_cpu: [KeyGpu; 128],
    active_cpu: Vec<u32>,
    stats: FrameStats,
}

impl Renderer {
    /// `format` is the final target's format: the swapchain for preview, an
    /// RGBA8 texture for export.
    pub fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        supersample: u32,
    ) -> Self {
        let d = &gpu.device;
        let scene_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                uniform_entry(0),
                storage_entry(1),
                storage_entry(2),
                storage_entry(3),
                storage_entry(4),
                uniform_entry(5),
                texture_entry(6),
                sampler_entry(7),
            ],
        });
        let bloom_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[texture_entry(0), sampler_entry(1), uniform_entry(2)],
        });
        let composite_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
            entries: &[texture_entry(0), texture_entry(1), sampler_entry(2), uniform_entry(3)],
        });
        let pipes = Pipelines::new(d, &scene_bgl, &bloom_bgl, &composite_bgl, format);
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform = |label, size: usize| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let storage = |label, size: u64| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let globals_buf = uniform("globals", std::mem::size_of::<Globals>());
        let particles_buf = uniform("particles", std::mem::size_of::<uniforms::ParticlesGpu>());
        let post_buf = uniform("post", std::mem::size_of::<PostGpu>());
        let notes_buf = storage("notes", 16);
        let tracks_buf = storage("tracks", (MAX_TRACKS * 16) as u64);
        let keys_buf = storage("keys", (128 * std::mem::size_of::<KeyGpu>()) as u64);
        let active_capacity = 4096;
        let active_buf = storage("active", active_capacity as u64 * 4);
        let bg_view = image::white_pixel(gpu);

        let design = Design::default();
        let score = Arc::new(Score::default());
        let layout = KeyLayout::new(21, 108, design.keyboard.black_width_ratio);
        let supersample = clamp_supersample(gpu, width, height, supersample);
        let frame =
            Frame::new(&design, (width * supersample) as f32, (height * supersample) as f32);
        let scene_bg = Self::make_scene_bg(
            d,
            &scene_bgl,
            [&globals_buf, &notes_buf, &tracks_buf, &keys_buf, &active_buf, &particles_buf],
            &bg_view,
            &sampler,
        );
        let targets = Targets::new(
            gpu,
            &bloom_bgl,
            &composite_bgl,
            &sampler,
            &post_buf,
            width,
            height,
            supersample,
        );

        let mut r = Self {
            format,
            width,
            height,
            supersample,
            scene_bgl,
            bloom_bgl,
            composite_bgl,
            pipes,
            sampler,
            globals_buf,
            particles_buf,
            post_buf,
            notes_buf,
            tracks_buf,
            keys_buf,
            active_buf,
            active_capacity,
            bg_view,
            scene_bg,
            targets,
            globals: uniforms::globals(&design, &frame, &layout, 60, None),
            post: PostGpu::default(),
            score,
            design,
            layout,
            frame,
            max_life: [0.0; 3],
            stride: 0,
            hands: None,
            image_aspect: None,
            track_visible: Vec::new(),
            track_colors: Vec::new(),
            keys_cpu: [KeyGpu::default(); 128],
            active_cpu: Vec::with_capacity(4096),
            stats: FrameStats::default(),
        };
        r.rebuild(gpu);
        r
    }

    fn make_scene_bg(
        d: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        [globals, notes, tracks, keys, active, particles]: [&wgpu::Buffer; 6],
        bg: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: notes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: tracks.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: keys.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: active.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: particles.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(bg),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn refresh_scene_bg(&mut self, d: &wgpu::Device) {
        self.scene_bg = Self::make_scene_bg(
            d,
            &self.scene_bgl,
            [
                &self.globals_buf,
                &self.notes_buf,
                &self.tracks_buf,
                &self.keys_buf,
                &self.active_buf,
                &self.particles_buf,
            ],
            &self.bg_view,
            &self.sampler,
        );
    }

    /// Load a score: uploads every note once. Nothing is re-uploaded per frame.
    pub fn set_score(&mut self, gpu: &Gpu, score: Arc<Score>) {
        let notes = score.notes.as_slice();
        let bytes: &[u8] = if notes.is_empty() { &[0; 16] } else { bytemuck::cast_slice(notes) };
        self.notes_buf = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("notes"),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        self.score = score;
        self.track_visible = vec![true; self.score.tracks.len()];
        self.track_colors = vec![None; self.score.tracks.len()];
        self.refresh_scene_bg(&gpu.device);
        self.rebuild(gpu);
    }

    /// Replace the notes of a score that changes every frame (live input),
    /// reusing the note buffer while it has room. Unlike [`set_score`],
    /// keeps track visibility and colors.
    ///
    /// [`set_score`]: Self::set_score
    pub fn update_score(&mut self, gpu: &Gpu, score: Arc<Score>) {
        let notes = score.notes.as_slice();
        let needed = (notes.len().max(1) * 16) as u64;
        if needed > self.notes_buf.size() {
            self.notes_buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("notes"),
                size: needed.next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.refresh_scene_bg(&gpu.device);
        }
        if !notes.is_empty() {
            gpu.queue.write_buffer(&self.notes_buf, 0, bytemuck::cast_slice(notes));
        }
        let auto = self.design.keyboard.range == pv_design::KeyRange::Auto;
        self.score = score;
        if auto {
            self.rebuild(gpu);
        }
    }

    /// Apply a Design. Returns problems worth showing (a missing background
    /// image, say) without failing: the rest of the Design still applies.
    pub fn set_design(
        &mut self,
        gpu: &Gpu,
        design: &Design,
        bundle_dir: Option<&Path>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        let image_changed = design.background.image != self.design.background.image
            || design.background.kind != self.design.background.kind
            || self.image_aspect.is_none();
        self.design = design.clone();
        if image_changed {
            self.image_aspect = None;
            self.bg_view = image::white_pixel(gpu);
            if design.background.kind == pv_design::BackgroundKind::Image
                && !design.background.image.is_empty()
            {
                match image::load_background(gpu, bundle_dir, &design.background.image) {
                    Ok((view, aspect)) => {
                        self.bg_view = view;
                        self.image_aspect = Some(aspect);
                    }
                    Err(e) => warnings.push(e),
                }
            }
            self.refresh_scene_bg(&gpu.device);
        }
        self.rebuild(gpu);
        warnings
    }

    pub fn design(&self) -> &Design {
        &self.design
    }

    pub fn score(&self) -> &Arc<Score> {
        &self.score
    }

    pub fn key_layout(&self) -> &KeyLayout {
        &self.layout
    }

    /// Show or hide tracks. Hidden notes aren't drawn and don't light keys.
    pub fn set_track_visibility(&mut self, gpu: &Gpu, visible: &[bool]) {
        self.track_visible = visible.to_vec();
        self.upload_tracks(gpu);
    }

    /// Per-track color overrides; `None` uses the Design's palette.
    pub fn set_track_colors(&mut self, gpu: &Gpu, colors: &[Option<Color>]) {
        self.track_colors = colors.to_vec();
        self.upload_tracks(gpu);
    }

    /// The color a track's notes are drawn in, for the UI swatch.
    ///
    /// Palette entries go to tracks in order among those that have notes, so
    /// a conductor track or a lyrics track doesn't use up the first color.
    pub fn track_color(&self, track: usize) -> Color {
        let pal = &self.design.notes.color.palette;
        let rank = match self.hands {
            Some((left, _)) => usize::from(track != left),
            None => self.score.visible_tracks().position(|(i, _)| i == track).unwrap_or(track),
        };
        self.track_colors
            .get(track)
            .copied()
            .flatten()
            .unwrap_or_else(|| pal.get(rank % pal.len().max(1)).copied().unwrap_or(Color::WHITE))
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32, supersample: u32) {
        let supersample = clamp_supersample(gpu, width, height, supersample);
        if (width, height, supersample) == (self.width, self.height, self.supersample) {
            return;
        }
        self.width = width.max(1);
        self.height = height.max(1);
        self.supersample = supersample;
        self.targets = Targets::new(
            gpu,
            &self.bloom_bgl,
            &self.composite_bgl,
            &self.sampler,
            &self.post_buf,
            self.width,
            self.height,
            supersample,
        );
        self.rebuild(gpu);
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// Recompute everything derived from Design, score, and size.
    fn rebuild(&mut self, gpu: &Gpu) {
        let ss = self.supersample;
        self.frame = Frame::new(&self.design, (self.width * ss) as f32, (self.height * ss) as f32);
        self.layout = KeyLayout::for_range(
            self.design.keyboard.range,
            &self.score.notes,
            self.design.keyboard.black_width_ratio,
        );
        self.globals = uniforms::globals(
            &self.design,
            &self.frame,
            &self.layout,
            self.score.suggested_split(),
            self.image_aspect,
        );
        self.globals.view[3] = ss as f32;
        // Piano MIDI usually has one track per hand; when it does, the
        // tracks say which hand played what far better than any pitch split.
        self.hands = None;
        if self.design.notes.color.source == pv_design::ColorSource::Hand
            && self.design.notes.color.split.is_none()
        {
            let two: Vec<_> = self.score.visible_tracks().take(3).collect();
            if let [(a, ta), (b, tb)] = two[..] {
                let mid = |t: &pv_core::TrackInfo| {
                    t.pitch_range.map_or(60, |(lo, hi)| lo as u32 + hi as u32)
                };
                self.hands = Some(if mid(ta) <= mid(tb) { (a, b) } else { (b, a) });
                self.globals.keys[2] = 0; // color by track
            }
        }
        let (p, life) = uniforms::particles(&self.design);
        self.max_life = life;
        gpu.queue.write_buffer(&self.particles_buf, 0, bytemuck::bytes_of(&p));

        self.stride = p.info[1];
        let t = &self.targets;
        let b = &self.design.post.bloom;
        let src_sizes: Vec<[u32; 2]> = std::iter::once([self.width * ss, self.height * ss])
            .chain(t.bloom.iter().map(|(_, s)| *s))
            .collect();
        for (i, buf) in t.down_bufs.iter().enumerate() {
            let s = src_sizes[i];
            let pass = BloomPass {
                texel: [1.0 / s[0] as f32, 1.0 / s[1] as f32],
                weight: 1.0,
                prefilter: if i == 0 { 1.0 } else { 0.0 },
                threshold: b.threshold,
                knee: b.soft_knee,
                _pad: [0.0; 2],
            };
            gpu.queue.write_buffer(buf, 0, bytemuck::bytes_of(&pass));
        }
        for (i, buf) in t.up_bufs.iter().enumerate() {
            let s = t.bloom[i + 1].1;
            let pass = BloomPass {
                texel: [1.0 / s[0] as f32, 1.0 / s[1] as f32],
                weight: 0.35 + 0.65 * b.radius,
                ..Default::default()
            };
            gpu.queue.write_buffer(buf, 0, bytemuck::bytes_of(&pass));
        }
        self.upload_tracks(gpu);
    }

    fn upload_tracks(&mut self, gpu: &Gpu) {
        let mut data = [[0.0f32; 4]; MAX_TRACKS];
        for (i, slot) in data.iter_mut().enumerate() {
            let [r, g, b, _] = self.track_color(i).linear();
            let visible = self.track_visible.get(i).copied().unwrap_or(true);
            *slot = [r, g, b, if visible { 1.0 } else { 0.0 }];
        }
        gpu.queue.write_buffer(&self.tracks_buf, 0, bytemuck::cast_slice(&data));
    }

    /// Record one frame into `encoder`, drawing into `target`.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        p: FrameParams,
    ) {
        let t = p.time as f32;
        let notes = &self.score.notes;

        // ── CPU: bounded by what's visible ──
        key_states(notes, t, &self.track_visible, &self.layout, &mut self.keys_cpu);
        active_notes(
            notes,
            t,
            self.max_life,
            &self.track_visible,
            &self.layout,
            &mut self.active_cpu,
        );
        let cam = camera_offset(&self.design, notes, t, self.frame.px);
        let look = self.design.layout.lookahead * 1.1 / self.design.camera.zoom.min(1.0);
        let range = match self.design.layout.direction {
            Direction::Down => notes.overlapping(t - 0.05, t + look),
            Direction::Up => notes.overlapping(t - look, t + 0.05),
        };

        // ── Upload: a few hundred bytes plus the active list ──
        self.globals.view[0] = t;
        self.globals.camera[0] = cam[0];
        self.globals.camera[1] = cam[1];
        gpu.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&self.globals));
        gpu.queue.write_buffer(&self.keys_buf, 0, bytemuck::cast_slice(&self.keys_cpu));
        if self.active_cpu.len() as u32 > self.active_capacity {
            self.active_capacity = (self.active_cpu.len() as u32).next_power_of_two();
            self.active_buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("active"),
                size: self.active_capacity as u64 * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.refresh_scene_bg(&gpu.device);
        }
        if !self.active_cpu.is_empty() {
            gpu.queue.write_buffer(&self.active_buf, 0, bytemuck::cast_slice(&self.active_cpu));
        }
        let encode_srgb = !self.format.is_srgb();
        self.post = uniforms::post(
            &self.design,
            &self.frame,
            self.width,
            self.height,
            self.supersample,
            p.alpha,
            encode_srgb,
        );
        self.post.vignette[3] = t;
        gpu.queue.write_buffer(&self.post_buf, 0, bytemuck::bytes_of(&self.post));

        let particle_instances = self.active_cpu.len() as u32 * self.stride;
        self.stats = FrameStats {
            visible_notes: range.len() as u32,
            particle_notes: self.active_cpu.len() as u32,
            particle_instances,
        };

        // ── Scene, in HDR ──
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.scene,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_bg, &[]);
            if !p.alpha {
                pass.set_pipeline(&self.pipes.background);
                pass.draw(0..3, 0..1);
            }
            if !range.is_empty() {
                let r = range.start as u32..range.end as u32;
                pass.set_pipeline(&self.pipes.notes_white);
                pass.draw(0..6, r.clone());
                pass.set_pipeline(&self.pipes.notes_black);
                pass.draw(0..6, r);
            }
            if self.frame.kb_h > 0.0 {
                let keys = self.layout.lo as u32..self.layout.hi as u32 + 1;
                pass.set_pipeline(&self.pipes.keys_white);
                pass.draw(0..6, keys.clone());
                pass.set_pipeline(&self.pipes.keys_black);
                pass.draw(0..6, keys.clone());
                pass.set_pipeline(&self.pipes.glow);
                pass.draw(0..6, keys);
            }
            if self.design.keyboard.strike_line.enabled {
                pass.set_pipeline(&self.pipes.line);
                pass.draw(0..6, 0..1);
            }
            if particle_instances > 0 {
                pass.set_pipeline(&self.pipes.particles);
                pass.draw(0..6, 0..particle_instances);
            }
        }

        // ── Bloom ──
        if self.design.post.bloom.enabled {
            let t = &self.targets;
            for (i, bg) in t.down.iter().enumerate() {
                fullscreen(encoder, &self.pipes.bloom_down, bg, &t.bloom[i].0, true);
            }
            for i in (0..t.up.len()).rev() {
                fullscreen(encoder, &self.pipes.bloom_up, &t.up[i], &t.bloom[i].0, false);
            }
        }

        // ── Composite to the target ──
        fullscreen(encoder, &self.pipes.composite, &self.targets.composite, target, true);
    }
}

fn clamp_supersample(gpu: &Gpu, w: u32, h: u32, ss: u32) -> u32 {
    let max = gpu.max_texture_size();
    let mut s = ss.clamp(1, 4);
    while s > 1 && (w * s > max || h * s > max) {
        s -= 1;
    }
    s
}

fn fullscreen(
    encoder: &mut wgpu::CommandEncoder,
    pipe: &wgpu::RenderPipeline,
    bg: &wgpu::BindGroup,
    target: &wgpu::TextureView,
    clear: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipe);
    pass.set_bind_group(0, bg, &[]);
    pass.draw(0..3, 0..1);
}

impl Targets {
    #[allow(clippy::too_many_arguments)]
    fn new(
        gpu: &Gpu,
        bloom_bgl: &wgpu::BindGroupLayout,
        composite_bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        post_buf: &wgpu::Buffer,
        width: u32,
        height: u32,
        ss: u32,
    ) -> Self {
        let d = &gpu.device;
        let tex = |label, w: u32, h: u32| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
        };
        let scene = tex("scene", width * ss, height * ss);

        // Bloom chain at half the *output* size, so the glow looks the same
        // whatever the supersampling.
        let mut bloom = Vec::new();
        let (mut w, mut h) = ((width / 2).max(1), (height / 2).max(1));
        for _ in 0..BLOOM_LEVELS {
            bloom.push((tex("bloom", w, h), [w, h]));
            if w <= 8 || h <= 8 {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }

        let pass_buf = || {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bloom pass"),
                size: std::mem::size_of::<BloomPass>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let bloom_bg = |src: &wgpu::TextureView, buf: &wgpu::Buffer| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom"),
                layout: bloom_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
                ],
            })
        };
        let down_bufs: Vec<_> = bloom.iter().map(|_| pass_buf()).collect();
        let up_bufs: Vec<_> = bloom.iter().skip(1).map(|_| pass_buf()).collect();
        let down = (0..bloom.len())
            .map(|i| bloom_bg(if i == 0 { &scene } else { &bloom[i - 1].0 }, &down_bufs[i]))
            .collect();
        let up = (0..bloom.len() - 1).map(|i| bloom_bg(&bloom[i + 1].0, &up_bufs[i])).collect();
        let composite = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite"),
            layout: composite_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom[0].0),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry { binding: 3, resource: post_buf.as_entire_binding() },
            ],
        });
        Self { scene, bloom, down, up, down_bufs, up_bufs, composite }
    }
}

impl Pipelines {
    fn new(
        d: &wgpu::Device,
        scene_bgl: &wgpu::BindGroupLayout,
        bloom_bgl: &wgpu::BindGroupLayout,
        composite_bgl: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let common = include_str!("shaders/common.wgsl");
        let module = |label, body: &str| {
            d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{common}\n{body}").into()),
            })
        };
        let bg = module("background", include_str!("shaders/background.wgsl"));
        let notes = module("notes", include_str!("shaders/notes.wgsl"));
        let keys = module("keyboard", include_str!("shaders/keyboard.wgsl"));
        let parts = module("particles", include_str!("shaders/particles.wgsl"));
        let bloom = d.create_shader_module(wgpu::include_wgsl!("shaders/bloom.wgsl"));
        let comp = d.create_shader_module(wgpu::include_wgsl!("shaders/composite.wgsl"));

        let layout = |bgl| {
            d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            })
        };
        let scene_l = layout(scene_bgl);
        let bloom_l = layout(bloom_bgl);
        let comp_l = layout(composite_bgl);

        let premul = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let add = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let black = [("BLACK_PASS", 1.0)];
        let make = |l: &wgpu::PipelineLayout,
                    m: &wgpu::ShaderModule,
                    vs: &str,
                    fs: &str,
                    fmt: wgpu::TextureFormat,
                    blend: Option<wgpu::BlendState>,
                    constants: &[(&str, f64)]| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(l),
                vertex: wgpu::VertexState {
                    module: m,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: m,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: fmt,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            background: make(&scene_l, &bg, "vs", "fs", HDR, None, &[]),
            notes_white: make(&scene_l, &notes, "vs", "fs", HDR, premul, &[]),
            notes_black: make(&scene_l, &notes, "vs", "fs", HDR, premul, &black),
            keys_white: make(&scene_l, &keys, "vs_key", "fs_key", HDR, premul, &[]),
            keys_black: make(&scene_l, &keys, "vs_key", "fs_key", HDR, premul, &black),
            glow: make(&scene_l, &keys, "vs_glow", "fs_glow", HDR, add, &[]),
            line: make(&scene_l, &keys, "vs_line", "fs_line", HDR, add, &[]),
            particles: make(&scene_l, &parts, "vs", "fs", HDR, add, &[]),
            bloom_down: make(&bloom_l, &bloom, "vs", "fs_down", HDR, None, &[]),
            bloom_up: make(&bloom_l, &bloom, "vs", "fs_up", HDR, add, &[]),
            composite: make(&comp_l, &comp, "vs", "fs", format, None, &[]),
        }
    }
}

fn vis() -> wgpu::ShaderStages {
    wgpu::ShaderStages::VERTEX_FRAGMENT
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis(),
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis(),
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis(),
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis(),
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
