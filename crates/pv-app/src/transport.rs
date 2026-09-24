//! Play, pause, seek — through the audio engine when there is one, and
//! through a wall clock when there isn't (no sound device, or the piano
//! still being generated). The rest of the app can't tell which.

use pv_audio::{Engine, EngineClock, SequenceOptions, SoundFont};
use pv_core::{Clock, Score};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

pub struct Transport {
    engine: Option<(Engine, EngineClock)>,
    /// The built-in piano, arriving from a worker thread.
    pending: Option<Receiver<Arc<SoundFont>>>,
    font: Option<Arc<SoundFont>>,
    /// Wall-clock fallback: position at `since`, advancing while playing.
    origin: f64,
    since: Option<Instant>,
    speed: f64,
    volume: f32,
    muted: Vec<bool>,
    options: SequenceOptions,
    /// Why there's no sound, for the status bar.
    pub audio_status: String,
    /// Start playing as soon as audio is ready (first run).
    autoplay: bool,
    low_latency: bool,
}

impl Transport {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(pv_audio::builtin_font());
        });
        Self {
            engine: None,
            pending: Some(rx),
            font: None,
            origin: 0.0,
            since: None,
            speed: 1.0,
            volume: 1.0,
            muted: Vec::new(),
            options: SequenceOptions::default(),
            audio_status: "preparing piano…".into(),
            autoplay: false,
            low_latency: false,
        }
    }

    /// Call once per frame: picks up the generated piano, frees audio garbage.
    pub fn update(&mut self, score: &Score) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(font) => {
                    self.pending = None;
                    self.use_font(font, score);
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.audio_status = "piano generation failed".into();
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some((e, _)) = &mut self.engine {
            e.collect_garbage();
        }
    }

    fn use_font(&mut self, font: Arc<SoundFont>, score: &Score) {
        let position = self.now();
        let playing = self.is_playing() || self.autoplay;
        self.autoplay = false;
        self.font = Some(font.clone());
        if let Some((e, _)) = &mut self.engine {
            if let Err(err) = e.set_font(&font) {
                self.audio_status = err.to_string();
            }
            return;
        }
        match Engine::start(font, self.low_latency.then_some(256)) {
            Ok(mut e) => {
                self.options.skip_percussion = !pv_audio::has_drums(self.font.as_ref().unwrap());
                e.set_score(score, self.options);
                e.set_volume(self.volume);
                e.set_muted(&self.muted);
                e.set_speed(self.speed);
                e.seek(position);
                if playing {
                    e.play();
                }
                self.audio_status = e.device_name().to_owned();
                let clock = e.clock();
                self.engine = Some((e, clock));
                self.since = None;
            }
            Err(err) => {
                // Keep going silently: the visuals still work.
                self.audio_status = format!("no sound: {err}");
                if playing && self.since.is_none() {
                    self.origin = position;
                    self.since = Some(Instant::now());
                }
            }
        }
    }

    pub fn font(&self) -> Option<&Arc<SoundFont>> {
        self.font.as_ref()
    }

    pub fn audio_ready(&self) -> bool {
        self.pending.is_none()
    }

    /// Start as soon as audio is ready, or now if it never will be.
    pub fn play_when_ready(&mut self) {
        if self.pending.is_some() {
            self.autoplay = true;
        } else {
            self.play();
        }
    }

    pub fn set_font(&mut self, font: Arc<SoundFont>, score: &Score) {
        self.use_font(font, score);
    }

    pub fn set_score(&mut self, score: &Score, muted: &[bool]) {
        self.muted = muted.to_vec();
        self.pause();
        self.origin = 0.0;
        if let Some((e, _)) = &mut self.engine {
            e.set_score(score, self.options);
            e.set_muted(muted);
        }
    }

    pub fn set_options(&mut self, options: SequenceOptions, score: &Score) {
        self.options = options;
        let (pos, playing) = (self.now(), self.is_playing());
        if let Some((e, _)) = &mut self.engine {
            e.set_score(score, options);
            e.set_muted(&self.muted);
            e.seek(pos);
            if playing {
                e.play();
            }
        }
    }

    pub fn options(&self) -> SequenceOptions {
        self.options
    }

    pub fn now(&self) -> f64 {
        match (&self.engine, self.since) {
            (Some((_, clock)), _) => clock.now(),
            (None, Some(t)) => self.origin + t.elapsed().as_secs_f64() * self.speed,
            (None, None) => self.origin,
        }
    }

    pub fn is_playing(&self) -> bool {
        match &self.engine {
            Some((e, _)) => e.is_playing(),
            None => self.since.is_some() || self.autoplay,
        }
    }

    pub fn play(&mut self) {
        match &mut self.engine {
            Some((e, _)) => e.play(),
            None => {
                if self.since.is_none() {
                    self.since = Some(Instant::now());
                }
            }
        }
    }

    pub fn pause(&mut self) {
        self.autoplay = false;
        match &mut self.engine {
            Some((e, _)) => e.pause(),
            None => {
                self.origin = self.now();
                self.since = None;
            }
        }
    }

    pub fn toggle(&mut self) {
        if self.is_playing() { self.pause() } else { self.play() }
    }

    pub fn seek(&mut self, t: f64) {
        let t = t.max(0.0);
        match &mut self.engine {
            Some((e, _)) => e.seek(t),
            None => {
                self.origin = t;
                if self.since.is_some() {
                    self.since = Some(Instant::now());
                }
            }
        }
    }

    pub fn set_speed(&mut self, speed: f64) {
        let pos = self.now();
        self.speed = speed.clamp(0.25, 2.0);
        match &mut self.engine {
            Some((e, _)) => e.set_speed(self.speed),
            None => self.seek(pos),
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v;
        if let Some((e, _)) = &mut self.engine {
            e.set_volume(v);
        }
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_muted(&mut self, muted: &[bool]) {
        self.muted = muted.to_vec();
        if let Some((e, _)) = &mut self.engine {
            e.set_muted(muted);
        }
    }

    pub fn latency(&self) -> Option<f64> {
        self.engine.as_ref().map(|(e, _)| e.latency())
    }

    /// The engine, when there is one: live input talks to it directly.
    pub fn engine(&mut self) -> Option<&mut Engine> {
        self.engine.as_mut().map(|(e, _)| e)
    }

    /// Reopen the audio device with a smaller buffer for live play (256
    /// frames, ~5 ms) or the playback default. A smaller buffer halves
    /// latency and doubles underrun risk: the right trade when someone is
    /// playing, stated rather than hidden.
    pub fn set_low_latency(&mut self, low: bool, score: &Score) {
        self.low_latency = low;
        let Some(font) = self.font.clone() else { return };
        if self.engine.is_some() {
            let pos = self.now();
            self.engine = None;
            self.use_font(font, score);
            self.seek(pos);
        }
    }

    pub fn low_latency(&self) -> bool {
        self.low_latency
    }
}
