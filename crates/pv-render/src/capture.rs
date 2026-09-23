//! Reading rendered frames back to the CPU. Export has its own pipelined
//! ring of buffers; this is the simple blocking path for tests, the CLI's
//! stills, and thumbnails.

use crate::{FrameParams, Gpu, Renderer};

/// Target format for everything read back: 8-bit RGBA, sRGB-encoded by the
/// composite shader itself so dithering happens in display space.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// wgpu requires each copied row to start on a 256-byte boundary.
pub fn padded_row(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// An offscreen RGBA8 target of a given size.
pub struct Offscreen {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Offscreen {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view, width, height }
    }

    /// Record a copy of this texture into `buffer`, which must hold
    /// `padded_row(width) * height` bytes.
    pub fn copy_to(&self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer) {
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row(self.width)),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
    }
}

/// Strip row padding: `padded` holds rows of `padded_row(width)` bytes.
pub fn unpad(padded: &[u8], width: u32, height: u32, out: &mut Vec<u8>) {
    let row = (width * 4) as usize;
    let stride = padded_row(width) as usize;
    out.clear();
    out.reserve(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&padded[y * stride..y * stride + row]);
    }
}

/// Render one frame and read it back as tightly packed RGBA8. Blocking.
pub fn render_image(
    gpu: &Gpu,
    renderer: &mut Renderer,
    target: &Offscreen,
    params: FrameParams,
) -> Vec<u8> {
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture"),
        size: (padded_row(target.width) * target.height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render(gpu, &mut encoder, &target.view, params);
    target.copy_to(&mut encoder, &buffer);
    gpu.queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |r| r.expect("map capture buffer"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU poll");
    let mut out = Vec::new();
    {
        let data = buffer.get_mapped_range(..).expect("mapped range");
        unpad(&data, target.width, target.height, &mut out);
    }
    buffer.unmap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_pad_to_256_bytes() {
        assert_eq!(padded_row(64), 256);
        assert_eq!(padded_row(1920), 7680);
        assert_eq!(padded_row(1921), 7936);
        assert_eq!(padded_row(1), 256);
    }

    #[test]
    fn unpad_drops_row_padding() {
        let (w, h) = (3u32, 2u32);
        let stride = padded_row(w) as usize;
        let mut padded = vec![0xEE; stride * h as usize];
        for y in 0..h as usize {
            for x in 0..(w * 4) as usize {
                padded[y * stride + x] = (y * 100 + x) as u8;
            }
        }
        let mut out = Vec::new();
        unpad(&padded, w, h, &mut out);
        assert_eq!(out.len(), 24);
        assert!(!out.contains(&0xEE));
        assert_eq!(out[12], 100);
    }
}
