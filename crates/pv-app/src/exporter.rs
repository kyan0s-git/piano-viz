//! Runs an export on a worker thread and reports back to the UI.

use pv_export::{Job, Stage, Summary};
use pv_render::Gpu;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// Widest thumbnail sent to the UI; full frames would be megabytes each.
const THUMB_WIDTH: u32 = 480;

pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

enum Update {
    Progress { stage: Stage, frame: u64, total: u64, eta: Option<Duration> },
    Thumb(Thumbnail),
    Done(Result<Summary, String>),
}

pub struct ExportRun {
    rx: Receiver<Update>,
    cancel: Arc<AtomicBool>,
    pub stage: Stage,
    pub frame: u64,
    pub total: u64,
    pub eta: Option<Duration>,
    pub thumbnail: Option<Thumbnail>,
    pub thumbnail_fresh: bool,
    pub result: Option<Result<Summary, String>>,
    pub output: std::path::PathBuf,
}

impl ExportRun {
    pub fn start(gpu: Arc<Gpu>, job: Job) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let output = job.settings.output.clone();
        let (w, h) = (job.settings.width, job.settings.height);
        std::thread::spawn(move || {
            let result = pv_export::export(&gpu, &job, |p| {
                let _ = tx.send(Update::Progress {
                    stage: p.stage,
                    frame: p.frame,
                    total: p.total,
                    eta: p.eta,
                });
                if let Some(px) = p.preview {
                    let _ = tx.send(Update::Thumb(downscale(px, w, h)));
                }
                !stop.load(Ordering::Relaxed)
            });
            let _ = tx.send(Update::Done(result.map_err(|e| e.to_string())));
        });
        Self {
            rx,
            cancel,
            stage: Stage::Audio,
            frame: 0,
            total: 1,
            eta: None,
            thumbnail: None,
            thumbnail_fresh: false,
            result: None,
            output,
        }
    }

    pub fn poll(&mut self) {
        while let Ok(u) = self.rx.try_recv() {
            match u {
                Update::Progress { stage, frame, total, eta } => {
                    (self.stage, self.frame, self.total, self.eta) =
                        (stage, frame, total.max(1), eta)
                }
                Update::Thumb(t) => {
                    self.thumbnail = Some(t);
                    self.thumbnail_fresh = true;
                }
                Update::Done(r) => self.result = Some(r),
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn fraction(&self) -> f32 {
        self.frame as f32 / self.total as f32
    }
}

/// Box-filter downscale by an integer factor.
fn downscale(rgba: &[u8], w: u32, h: u32) -> Thumbnail {
    let f = w.div_ceil(THUMB_WIDTH).max(1);
    let (tw, th) = (w / f, h / f);
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            let mut acc = [0u32; 4];
            for dy in 0..f {
                for dx in 0..f {
                    let i = (((y * f + dy) * w + x * f + dx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += rgba[i + c] as u32;
                    }
                }
            }
            out.extend(acc.map(|v| (v / (f * f)) as u8));
        }
    }
    Thumbnail { width: tw, height: th, rgba: out }
}
