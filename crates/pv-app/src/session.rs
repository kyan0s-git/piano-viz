//! What's open: the score, the Design, per-track state, the loop. Not part
//! of a Design, because sharing a look shouldn't share someone's mutes and
//! playhead (docs/02-architecture.md).

use notify::Watcher;
use pv_core::Score;
use pv_design::{Color, Design};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum DesignSource {
    Builtin(&'static str),
    File(PathBuf),
    /// Edited since it was loaded, and not saved.
    Unsaved,
}

pub struct Message {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct Session {
    pub score: Arc<Score>,
    pub score_name: String,
    pub design: Design,
    pub design_source: DesignSource,
    pub bundle_dir: Option<PathBuf>,
    pub visible: Vec<bool>,
    pub muted: Vec<bool>,
    pub solo: Option<usize>,
    pub track_colors: Vec<Option<Color>>,
    pub loop_range: Option<(f64, f64)>,
    pub looping: bool,
    /// Frame shape the preview is letterboxed to: the export resolution.
    pub frame_size: (u32, u32),
    /// Note starts per timeline bucket, normalized 0..1.
    pub density: Vec<f32>,
    pub bars: Vec<f64>,
    pub messages: Vec<Message>,
    /// Set whenever the renderer needs the Design re-applied.
    pub design_dirty: bool,
    /// Set when track visibility, color, or mutes changed.
    pub tracks_dirty: bool,
    history: Vec<Design>,
    cursor: usize,
    watcher: Option<(notify::RecommendedWatcher, Receiver<notify::Result<notify::Event>>)>,
    reload_at: Option<Instant>,
}

impl Session {
    pub fn new(score: Score, name: &str) -> Self {
        let design = pv_design::builtin("ember-classic");
        let mut s = Self {
            score: Arc::new(Score::default()),
            score_name: String::new(),
            design: design.clone(),
            design_source: DesignSource::Builtin("ember-classic"),
            bundle_dir: None,
            visible: Vec::new(),
            muted: Vec::new(),
            solo: None,
            track_colors: Vec::new(),
            loop_range: None,
            looping: false,
            frame_size: (1920, 1080),
            density: Vec::new(),
            bars: Vec::new(),
            messages: Vec::new(),
            design_dirty: true,
            tracks_dirty: true,
            history: vec![design],
            cursor: 0,
            watcher: None,
            reload_at: None,
        };
        s.set_score(score, name);
        s
    }

    pub fn set_score(&mut self, score: Score, name: &str) {
        for w in &score.warnings {
            self.info(w.clone());
        }
        let n = score.tracks.len();
        self.visible = vec![true; n];
        self.muted = vec![false; n];
        self.track_colors = vec![None; n];
        self.solo = None;
        self.loop_range = None;
        self.looping = false;
        self.bars = score.tempo.bars(score.duration);
        self.density = density(&score, 800);
        self.score = Arc::new(score);
        self.score_name = name.to_owned();
        self.tracks_dirty = true;
        self.design_dirty = true;
    }

    /// Visibility after solo is applied.
    pub fn shown(&self) -> Vec<bool> {
        (0..self.visible.len())
            .map(|i| self.visible[i] && self.solo.is_none_or(|s| s == i))
            .collect()
    }

    /// Mutes after solo is applied.
    pub fn silenced(&self) -> Vec<bool> {
        (0..self.muted.len()).map(|i| self.muted[i] || self.solo.is_some_and(|s| s != i)).collect()
    }

    /// End of the song plus the time effects need to finish.
    pub fn end(&self) -> f64 {
        self.score.duration + pv_export::default_tail(&self.design)
    }

    // ── Design ──

    pub fn set_design(
        &mut self,
        design: Design,
        source: DesignSource,
        bundle_dir: Option<PathBuf>,
    ) {
        self.design = design;
        self.design_source = source;
        self.bundle_dir = bundle_dir;
        self.design_dirty = true;
        self.commit();
        self.watch();
    }

    pub fn load_design_file(&mut self, path: &Path) {
        match pv_design::load(path) {
            Ok(l) => {
                for w in &l.warnings {
                    self.info(format!("Design: {w}"));
                }
                self.set_design(l.design, DesignSource::File(path.to_path_buf()), l.bundle_dir);
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    /// Record the current Design as an undo step if it changed. Called when
    /// an edit ends (the mouse is released), so a drag is one step.
    pub fn commit(&mut self) {
        if self.history.get(self.cursor) != Some(&self.design) {
            self.history.truncate(self.cursor + 1);
            self.history.push(self.design.clone());
            if self.history.len() > 200 {
                self.history.remove(0);
            }
            self.cursor = self.history.len() - 1;
        }
    }

    pub fn edited(&mut self) {
        self.design_dirty = true;
        if let DesignSource::Builtin(_) = self.design_source {
            self.design_source = DesignSource::Unsaved;
        }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }

    pub fn undo(&mut self) {
        self.commit();
        if self.can_undo() {
            self.cursor -= 1;
            self.design = self.history[self.cursor].clone();
            self.design_dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if self.can_redo() {
            self.cursor += 1;
            self.design = self.history[self.cursor].clone();
            self.design_dirty = true;
        }
    }

    pub fn save_design(&mut self, path: &Path) {
        let mut d = self.design.clone();
        if d.meta.name == "Untitled"
            || matches!(self.design_source, DesignSource::Builtin(_) | DesignSource::Unsaved)
        {
            d.meta.name =
                path.file_stem().map_or("My Design".into(), |s| s.to_string_lossy().into_owned());
        }
        match std::fs::write(path, pv_design::to_toml(&d)) {
            Ok(()) => {
                self.design.meta.name = d.meta.name;
                self.design_source = DesignSource::File(path.to_path_buf());
                self.bundle_dir = path.parent().map(Path::to_path_buf);
                self.watch();
                self.info(format!("Saved {}", path.display()));
            }
            Err(e) => self.error(format!("{}: {e}", path.display())),
        }
    }

    /// Watch the Design file, so editing it in a text editor updates the
    /// preview mid-playback.
    fn watch(&mut self) {
        self.watcher = None;
        let DesignSource::File(path) = &self.design_source else { return };
        let file = if path.is_dir() { path.join("design.toml") } else { path.clone() };
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut w) = notify::recommended_watcher(tx) else { return };
        // Watch the directory: editors often save by replacing the file,
        // which drops a watch on the file itself.
        let dir = file.parent().unwrap_or(Path::new("."));
        if w.watch(dir, notify::RecursiveMode::NonRecursive).is_ok() {
            self.watcher = Some((w, rx));
        }
    }

    /// Per frame: reload the Design if its file changed.
    pub fn poll_reload(&mut self) {
        let DesignSource::File(path) = self.design_source.clone() else { return };
        let file = if path.is_dir() { path.join("design.toml") } else { path.clone() };
        if let Some((_, rx)) = &self.watcher {
            while let Ok(ev) = rx.try_recv() {
                if ev.is_ok_and(|e| e.paths.iter().any(|p| p.file_name() == file.file_name())) {
                    // Debounce: editors write in bursts.
                    self.reload_at = Some(Instant::now() + Duration::from_millis(120));
                }
            }
        }
        if self.reload_at.is_some_and(|t| Instant::now() >= t) {
            self.reload_at = None;
            match pv_design::load(&path) {
                Ok(l) => {
                    // Keep undo history continuous across external edits.
                    self.design = l.design;
                    self.bundle_dir = l.bundle_dir;
                    self.design_dirty = true;
                    self.commit();
                    self.info("Design reloaded".into());
                }
                // Keep the last good Design; say what's wrong and where.
                Err(e) => self.error(e.to_string()),
            }
        }
    }

    // ── Messages ──

    pub fn info(&mut self, text: String) {
        self.messages.push(Message { text, error: false, at: Instant::now() });
    }

    pub fn error(&mut self, text: String) {
        self.messages.push(Message { text, error: true, at: Instant::now() });
    }

    pub fn expire_messages(&mut self) {
        self.messages
            .retain(|m| m.at.elapsed() < Duration::from_secs(if m.error { 12 } else { 5 }));
    }
}

/// Note activity over the song, for the timeline strip. Square-root scaled
/// so quiet passages still show.
fn density(score: &Score, buckets: usize) -> Vec<f32> {
    let mut d = vec![0f32; buckets];
    let len = score.duration.max(1e-3);
    for n in score.notes.as_slice() {
        let i = ((n.start as f64 / len) * buckets as f64) as usize;
        if let Some(b) = d.get_mut(i) {
            *b += 1.0;
        }
    }
    let max = d.iter().fold(0f32, |m, &v| m.max(v)).max(1.0);
    d.iter().map(|v| (v / max).sqrt()).collect()
}
