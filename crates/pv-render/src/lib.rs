//! GPU renderer for piano-viz. See docs/04-rendering.md and
//! docs/05-particles.md.
//!
//! The renderer is a pure function of (score, design, time): given the same
//! inputs it draws the same frame, whichever clock supplied the time. That
//! is what lets preview and export share it.

mod capture;
mod frame;
mod gpu;
mod image;
mod layout;
mod renderer;
mod uniforms;

pub use capture::{CAPTURE_FORMAT, Offscreen, padded_row, render_image, unpad};
pub use gpu::Gpu;
pub use image::decode as decode_image;
pub use layout::{KeyGeom, KeyLayout};
pub use renderer::{FrameParams, FrameStats, Renderer};
pub use wgpu;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no usable GPU adapter: {0}")]
    NoAdapter(String),
}
