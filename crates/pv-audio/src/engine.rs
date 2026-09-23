//! Real-time playback. The audio callback owns the synthesizer and the
//! playhead; everything else talks to it through lock-free queues and reads
//! the playhead it publishes. See docs/06-audio.md.

use crate::sequencer::{Chase, Player, Sequence, SequenceOptions, TrackMask};
use crate::{AudioError, limit, new_synth};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pv_core::{Clock, Score};
use rustysynth::{SoundFont, Synthesizer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Largest slice rendered in one go; bigger callbacks loop.
const MAX_BLOCK: usize = 2048;

enum Command {
    Play,
    Pause,
    Seek { sample: u64, chase: Box<Chase> },
    Sequence(Arc<Sequence>),
    Synth(Box<Synthesizer>),
    Volume(f32),
    Mute(TrackMask),
    Speed(f64),
    NoteOn { channel: u8, key: u8, velocity: u8 },
    NoteOff { channel: u8, key: u8 },
    Controller { channel: u8, number: u8, value: u8 },
    AllOff,
}

/// Replaced objects travel back to the main thread to be freed, so the
/// audio thread never runs a destructor that might deallocate.
#[expect(dead_code, reason = "held only so they are dropped on the main thread")]
enum Trash {
    Sequence(Arc<Sequence>),
    Synth(Box<Synthesizer>),
    Chase(Box<Chase>),
}

struct Shared {
    /// Song position in samples at 1x speed, published after each callback.
    position: AtomicU64,
    playing: AtomicBool,
    /// Seconds from generating a sample to hearing it, as f64 bits.
    latency: AtomicU64,
    speed: AtomicU64,
    sample_rate: u32,
}

pub struct Engine {
    stream: cpal::Stream,
    commands: rtrb::Producer<Command>,
    trash: rtrb::Consumer<Trash>,
    shared: Arc<Shared>,
    sequence: Arc<Sequence>,
    options: SequenceOptions,
    device: String,
}

/// The playhead as the renderer should see it: what's being *heard*, which
/// trails what's been computed by the output latency.
#[derive(Clone)]
pub struct EngineClock(Arc<Shared>);

impl Clock for EngineClock {
    fn now(&self) -> f64 {
        let s = &self.0;
        let pos = s.position.load(Ordering::Acquire) as f64 / s.sample_rate as f64;
        if s.playing.load(Ordering::Acquire) {
            let speed = f64::from_bits(s.speed.load(Ordering::Relaxed));
            (pos - f64::from_bits(s.latency.load(Ordering::Relaxed)) * speed).max(0.0)
        } else {
            pos
        }
    }
}

impl Engine {
    /// Open the default output device. `buffer_frames` trades latency for
    /// underrun safety: 512 for playback, 256 for live play.
    pub fn start(font: Arc<SoundFont>, buffer_frames: Option<u32>) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::Device("no audio output device".into()))?;
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "audio output".into());
        let supported =
            device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
        let mut config = supported.config();
        if let Some(frames) = buffer_frames {
            config.buffer_size = cpal::BufferSize::Fixed(frames);
        }
        let sample_rate = config.sample_rate;
        let synth = new_synth(&font, sample_rate)?;

        let shared = Arc::new(Shared {
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            latency: AtomicU64::new(0f64.to_bits()),
            speed: AtomicU64::new(1f64.to_bits()),
            sample_rate,
        });
        let (commands, rx) = rtrb::RingBuffer::new(256);
        let (trash_tx, trash) = rtrb::RingBuffer::new(64);
        let sequence =
            Arc::new(Sequence::new(&Score::default(), sample_rate, SequenceOptions::default()));
        let state = Callback {
            rx,
            trash: trash_tx,
            shared: shared.clone(),
            synth: Box::new(synth),
            sequence: sequence.clone(),
            player: Player::default(),
            playing: false,
            volume: 1.0,
            channels: config.channels.max(1) as usize,
            left: vec![0.0; MAX_BLOCK],
            right: vec![0.0; MAX_BLOCK],
        };

        let err = |e| eprintln!("audio stream error: {e}");
        macro_rules! build {
            ($t:ty) => {{
                let mut state = state;
                device.build_output_stream::<$t, _, _>(
                    config,
                    move |data: &mut [$t], info: &cpal::OutputCallbackInfo| {
                        state.process(data, info)
                    },
                    err,
                    None,
                )
            }};
        }
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::I32 => build!(i32),
            f => return Err(AudioError::Device(format!("unsupported sample format {f:?}"))),
        }
        .map_err(|e| AudioError::Device(e.to_string()))?;
        stream.play().map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            stream,
            commands,
            trash,
            shared,
            sequence,
            options: SequenceOptions::default(),
            device: name,
        })
    }

    fn send(&mut self, c: Command) {
        // Full only if the audio thread has stalled; dropping a command then
        // is better than blocking the UI.
        let _ = self.commands.push(c);
    }

    /// Load a score. Stops playback and rewinds.
    pub fn set_score(&mut self, score: &Score, options: SequenceOptions) {
        self.options = options;
        self.sequence = Arc::new(Sequence::new(score, self.shared.sample_rate, options));
        self.send(Command::Pause);
        self.send(Command::Sequence(self.sequence.clone()));
        self.seek(0.0);
    }

    /// Swap the SoundFont. The synth is built here, not on the audio thread.
    pub fn set_font(&mut self, font: &Arc<SoundFont>) -> Result<(), AudioError> {
        let synth = new_synth(font, self.shared.sample_rate)?;
        self.send(Command::Synth(Box::new(synth)));
        let pos = self.position();
        self.seek(pos);
        Ok(())
    }

    pub fn play(&mut self) {
        self.send(Command::Play);
    }

    pub fn pause(&mut self) {
        self.send(Command::Pause);
    }

    pub fn seek(&mut self, seconds: f64) {
        let sample = (seconds.max(0.0) * self.shared.sample_rate as f64).round() as u64;
        // Publish at once so the picture jumps with the click, not a
        // callback later.
        self.shared.position.store(sample, Ordering::Release);
        let chase = Box::new(self.sequence.chase(sample));
        self.send(Command::Seek { sample, chase });
    }

    pub fn set_volume(&mut self, v: f32) {
        self.send(Command::Volume(v.clamp(0.0, 2.0)));
    }

    pub fn set_muted(&mut self, muted: &[bool]) {
        self.send(Command::Mute(TrackMask::from_muted(muted)));
    }

    pub fn set_speed(&mut self, speed: f64) {
        let s = speed.clamp(0.1, 4.0);
        self.shared.speed.store(s.to_bits(), Ordering::Relaxed);
        self.send(Command::Speed(s));
    }

    /// Live input: strike a key now.
    pub fn note_on(&mut self, channel: u8, key: u8, velocity: u8) {
        self.send(Command::NoteOn { channel, key, velocity });
    }

    pub fn note_off(&mut self, channel: u8, key: u8) {
        self.send(Command::NoteOff { channel, key });
    }

    pub fn controller(&mut self, channel: u8, number: u8, value: u8) {
        self.send(Command::Controller { channel, number, value });
    }

    pub fn all_notes_off(&mut self) {
        self.send(Command::AllOff);
    }

    /// Free what the audio thread handed back. Call once per UI frame.
    pub fn collect_garbage(&mut self) {
        while let Ok(t) = self.trash.pop() {
            drop(t);
        }
    }

    pub fn position(&self) -> f64 {
        self.shared.position.load(Ordering::Acquire) as f64 / self.shared.sample_rate as f64
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    pub fn clock(&self) -> EngineClock {
        EngineClock(self.shared.clone())
    }

    pub fn latency(&self) -> f64 {
        f64::from_bits(self.shared.latency.load(Ordering::Relaxed))
    }

    pub fn sample_rate(&self) -> u32 {
        self.shared.sample_rate
    }

    pub fn device_name(&self) -> &str {
        &self.device
    }

    pub fn options(&self) -> SequenceOptions {
        self.options
    }

    /// Stop producing sound while keeping the device open.
    pub fn suspend(&self) {
        let _ = self.stream.pause();
    }

    pub fn resume(&self) {
        let _ = self.stream.play();
    }
}

/// Everything the callback owns. Allocated before the stream starts.
struct Callback {
    rx: rtrb::Consumer<Command>,
    trash: rtrb::Producer<Trash>,
    shared: Arc<Shared>,
    synth: Box<Synthesizer>,
    sequence: Arc<Sequence>,
    player: Player,
    playing: bool,
    volume: f32,
    channels: usize,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Callback {
    fn process<T: cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        data: &mut [T],
        info: &cpal::OutputCallbackInfo,
    ) {
        self.handle_commands();

        let ts = info.timestamp();
        if let Some(lat) = ts.playback.checked_duration_since(ts.callback) {
            self.shared.latency.store(lat.as_secs_f64().to_bits(), Ordering::Relaxed);
        }

        let ch = self.channels;
        for frame_chunk in data.chunks_mut(MAX_BLOCK * ch) {
            let n = frame_chunk.len() / ch;
            let (l, r) = (&mut self.left[..n], &mut self.right[..n]);
            if self.playing {
                self.player.render(&self.sequence, &mut self.synth, l, r);
            } else {
                // Keep rendering while paused: released notes ring out and
                // live input still sounds.
                self.synth.render(l, r);
            }
            for (i, frame) in frame_chunk.chunks_exact_mut(ch).enumerate() {
                let (a, b) = (limit(l[i] * self.volume), limit(r[i] * self.volume));
                for (c, s) in frame.iter_mut().enumerate() {
                    *s = T::from_sample(match c {
                        0 => a,
                        1 => b,
                        _ => 0.0,
                    });
                }
                if ch == 1 {
                    frame[0] = T::from_sample((a + b) * 0.5);
                }
            }
        }
        if self.playing {
            self.shared.position.store(self.player.position(), Ordering::Release);
        }
    }

    fn handle_commands(&mut self) {
        while let Ok(cmd) = self.rx.pop() {
            match cmd {
                Command::Play => {
                    self.playing = true;
                    self.shared.playing.store(true, Ordering::Release);
                }
                Command::Pause => {
                    self.playing = false;
                    self.shared.playing.store(false, Ordering::Release);
                    self.synth.note_off_all(false);
                }
                Command::Seek { sample, chase } => {
                    self.player.seek(&self.sequence, &mut self.synth, sample, &chase);
                    self.shared.position.store(sample, Ordering::Release);
                    let _ = self.trash.push(Trash::Chase(chase));
                }
                Command::Sequence(seq) => {
                    let old = std::mem::replace(&mut self.sequence, seq);
                    let _ = self.trash.push(Trash::Sequence(old));
                }
                Command::Synth(synth) => {
                    let old = std::mem::replace(&mut self.synth, synth);
                    let _ = self.trash.push(Trash::Synth(old));
                }
                Command::Volume(v) => self.volume = v,
                Command::Mute(m) => self.player.muted = m,
                Command::Speed(s) => self.player.speed = s,
                Command::NoteOn { channel, key, velocity } => {
                    self.synth.note_on(channel as i32, key as i32, velocity as i32)
                }
                Command::NoteOff { channel, key } => {
                    self.synth.note_off(channel as i32, key as i32)
                }
                Command::Controller { channel, number, value } => self.synth.process_midi_message(
                    channel as i32,
                    0xB0,
                    number as i32,
                    value as i32,
                ),
                Command::AllOff => self.synth.note_off_all(false),
            }
        }
    }
}
