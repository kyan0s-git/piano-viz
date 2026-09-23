//! Background images: PNG and JPEG, decoded once when a Design is applied.

use crate::Gpu;
use std::path::Path;

/// A 1x1 white texture, bound when there's no background image.
pub fn white_pixel(gpu: &Gpu) -> wgpu::TextureView {
    upload(gpu, 1, 1, &[255, 255, 255, 255])
}

/// Returns the texture and its width/height aspect ratio.
pub fn load_background(
    gpu: &Gpu,
    bundle: Option<&Path>,
    rel: &str,
) -> Result<(wgpu::TextureView, f32), String> {
    let path = match bundle {
        Some(dir) => pv_design::resolve_asset(dir, rel).map_err(|e| e.to_string())?,
        None => std::path::PathBuf::from(rel),
    };
    let bytes =
        std::fs::read(&path).map_err(|e| format!("background image {}: {e}", path.display()))?;
    let (w, h, rgba) =
        decode(&bytes).map_err(|e| format!("background image {}: {e}", path.display()))?;
    let max = gpu.max_texture_size();
    if w > max || h > max {
        return Err(format!(
            "background image {} is {w}x{h}; the GPU allows at most {max}",
            path.display()
        ));
    }
    Ok((upload(gpu, w, h, &rgba), w as f32 / h as f32))
}

/// Decode PNG or JPEG to 8-bit RGBA.
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if bytes.starts_with(b"\x89PNG") {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        buf.truncate(info.buffer_size());
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => {
                buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
            }
            png::ColorType::GrayscaleAlpha => {
                buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
            }
            png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
            png::ColorType::Indexed => return Err("unexpected indexed PNG after expansion".into()),
        };
        return Ok((info.width, info.height, rgba));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
        let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut dec = zune_jpeg::JpegDecoder::new_with_options(bytes, opts);
        let rgba = dec.decode().map_err(|e| e.to_string())?;
        let (w, h) = dec.dimensions().ok_or("JPEG has no dimensions")?;
        return Ok((w as u32, h as u32, rgba));
    }
    Err("not a PNG or JPEG file".into())
}

fn upload(gpu: &Gpu, w: u32, h: u32, rgba: &[u8]) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    gpu.device
        .create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("background"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // sRGB so sampling returns linear light.
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        )
        .create_view(&Default::default())
}
