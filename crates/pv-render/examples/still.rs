//! Render single frames of the demo piece to PNG, for eyeballing changes.
//!
//! cargo run --release -p pv-render --example still -- <design> <out.png> <time>...

use pv_render::{FrameParams, Gpu, Offscreen, Renderer, render_image};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let design = pv_design::builtin(args.first().map_or("ember-classic", String::as_str));
    let out = args.get(1).map_or("still.png", String::as_str);
    let times: Vec<f64> = args.iter().skip(2).filter_map(|s| s.parse().ok()).collect();
    let (w, h) = (1280, 720);

    let gpu = Gpu::headless().expect("gpu");
    eprintln!("adapter: {}", gpu.describe());
    let score = Arc::new(pv_midi::parse(&pv_midi::demo::prelude()).unwrap());
    let mut r = Renderer::new(&gpu, pv_render::CAPTURE_FORMAT, w, h, 2);
    r.set_score(&gpu, score);
    r.set_design(&gpu, &design, None);
    let target = Offscreen::new(&gpu, w, h);
    for (i, &t) in times.iter().enumerate() {
        let start = std::time::Instant::now();
        let px = render_image(
            &gpu,
            &mut r,
            &target,
            FrameParams { time: t, frame: i as u64, alpha: false },
        );
        eprintln!("t={t}: {:?} {:?}", start.elapsed(), r.stats());
        let path = if times.len() == 1 {
            out.to_string()
        } else {
            out.replace(".png", &format!("-{i}.png"))
        };
        let file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let mut enc = png::Encoder::new(file, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&px).unwrap();
    }
}
