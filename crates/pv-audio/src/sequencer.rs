//! Turning a score into synth calls at the right sample.
//!
//! The event list is built once per score and sample rate. Playing it is a
//! forward cursor walk; seeking is a binary search plus a "chase" of the
//! controllers set before the seek point (pedal, program, bend), computed
//! off the audio thread and sent along with the seek.

use pv_core::{ControlKind, Score, flags};
use rustysynth::Synthesizer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Program { channel: u8, program: u8 },
    Controller { channel: u8, number: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    NoteOff { channel: u8, key: u8 },
    NoteOn { channel: u8, key: u8, velocity: u8, track: u8 },
}

impl EventKind {
    /// Same-sample ordering: setup first, then releases before strikes, so
    /// a key released and re-struck on one sample re-sounds.
    fn rank(&self) -> u8 {
        match self {
            Self::Program { .. } => 0,
            Self::Controller { .. } => 1,
            Self::PitchBend { .. } => 2,
            Self::NoteOff { .. } => 3,
            Self::NoteOn { .. } => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// Song position in samples at 1x speed.
    pub sample: u64,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug)]
pub struct SequenceOptions {
    /// Ignore program changes so every part plays on the piano. The right
    /// default for a piano visualizer; off to hear a GM file as authored.
    pub force_piano: bool,
    /// Drop channel 10 percussion, which would otherwise play as piano notes
    /// on a font with no drum kit.
    pub skip_percussion: bool,
}

impl Default for SequenceOptions {
    fn default() -> Self {
        Self { force_piano: true, skip_percussion: true }
    }
}

pub struct Sequence {
    pub events: Vec<Event>,
    pub sample_rate: u32,
    /// Last event, in samples.
    pub end: u64,
}

impl Sequence {
    pub fn new(score: &Score, sample_rate: u32, opts: SequenceOptions) -> Self {
        let sr = sample_rate as f64;
        let at = |t: f64| (t.max(0.0) * sr).round() as u64;
        let mut events = Vec::with_capacity(score.notes.len() * 2 + score.controls.len());
        for c in &score.controls {
            let channel = c.channel & 15;
            let kind = match c.kind {
                ControlKind::Program(_) if opts.force_piano => continue,
                // Bank select would move a forced piano off bank 0.
                ControlKind::Controller { number: 0 | 32, .. } if opts.force_piano => continue,
                ControlKind::Program(program) => EventKind::Program { channel, program },
                ControlKind::Controller { number, value } => {
                    EventKind::Controller { channel, number, value }
                }
                ControlKind::PitchBend(value) => EventKind::PitchBend { channel, value },
            };
            events.push(Event { sample: at(c.time), kind });
        }
        for n in score.notes.as_slice() {
            if opts.skip_percussion && n.has(flags::PERCUSSION) {
                continue;
            }
            let channel = n.channel();
            events.push(Event {
                sample: at(n.start as f64),
                kind: EventKind::NoteOn {
                    channel,
                    key: n.pitch,
                    velocity: n.velocity.max(1),
                    track: n.track,
                },
            });
            events.push(Event {
                sample: at(n.end() as f64),
                kind: EventKind::NoteOff { channel, key: n.pitch },
            });
        }
        events.sort_by_key(|e| (e.sample, e.kind.rank()));
        let end = events.last().map_or(0, |e| e.sample);
        Self { events, sample_rate, end }
    }

    /// Index of the first event at or after `sample`.
    pub fn index_at(&self, sample: u64) -> usize {
        self.events.partition_point(|e| e.sample < sample)
    }

    /// Controller state in effect just before `sample`, to restore on seek.
    pub fn chase(&self, sample: u64) -> Chase {
        let mut c = Chase::default();
        for e in &self.events[..self.index_at(sample)] {
            match e.kind {
                EventKind::Program { channel, program } => {
                    c.program[channel as usize] = program as i8
                }
                EventKind::Controller { channel, number, value }
                    if (number as usize) < CHASED_CCS =>
                {
                    c.cc[channel as usize][number as usize] = value as i8
                }
                EventKind::PitchBend { channel, value } => c.bend[channel as usize] = value,
                _ => {}
            }
        }
        c
    }
}

/// Controllers 0-119 are state; 120+ are channel-mode commands (all notes
/// off, reset), which must not be replayed.
const CHASED_CCS: usize = 120;

/// Per-channel controller state. -1 means never set. Plain data, so it
/// travels to the audio thread by value with no allocation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chase {
    pub program: [i8; 16],
    pub cc: [[i8; CHASED_CCS]; 16],
    pub bend: [i16; 16],
}

impl Default for Chase {
    fn default() -> Self {
        Self { program: [-1; 16], cc: [[-1; CHASED_CCS]; 16], bend: [0; 16] }
    }
}

/// 256 tracks' worth of mute bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackMask(pub [u64; 4]);

impl TrackMask {
    pub fn from_muted(muted: &[bool]) -> Self {
        let mut m = Self::default();
        for (i, _) in muted.iter().enumerate().filter(|(_, m)| **m).take(256) {
            m.0[i / 64] |= 1 << (i % 64);
        }
        m
    }

    #[inline]
    pub fn contains(&self, track: u8) -> bool {
        self.0[track as usize / 64] & (1 << (track % 64)) != 0
    }
}

/// Plays a [`Sequence`] into a synthesizer. Owned by whoever drives the
/// synth: the audio callback, or the offline renderer.
#[derive(Clone, Debug)]
pub struct Player {
    cursor: usize,
    /// Song position in samples at 1x speed, fractional so non-1x speeds
    /// don't accumulate rounding.
    position: f64,
    pub muted: TrackMask,
    pub speed: f64,
}

impl Default for Player {
    fn default() -> Self {
        Self { cursor: 0, position: 0.0, muted: TrackMask::default(), speed: 1.0 }
    }
}

impl Player {
    pub fn position(&self) -> u64 {
        self.position as u64
    }

    /// Jump to `sample`: silence everything, restore controllers, resume.
    /// Notes already sounding at the seek point are not re-struck.
    pub fn seek(&mut self, seq: &Sequence, synth: &mut Synthesizer, sample: u64, chase: &Chase) {
        synth.note_off_all(true);
        synth.reset_all_controllers();
        for ch in 0..16 {
            let c = ch as i32;
            if chase.program[ch] >= 0 {
                synth.process_midi_message(c, 0xC0, chase.program[ch] as i32, 0);
            }
            for (num, &v) in chase.cc[ch].iter().enumerate() {
                if v >= 0 {
                    synth.process_midi_message(c, 0xB0, num as i32, v as i32);
                }
            }
            let b = chase.bend[ch] as i32 + 8192;
            synth.process_midi_message(c, 0xE0, b & 127, b >> 7);
        }
        self.cursor = seq.index_at(sample);
        self.position = sample as f64;
    }

    /// Render `left.len()` output samples, applying every event that falls
    /// inside them. Events land on the synth's internal block boundary
    /// (64 samples, 1.3 ms at 48 kHz) — far below what's audible.
    pub fn render(
        &mut self,
        seq: &Sequence,
        synth: &mut Synthesizer,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let n = left.len();
        let mut done = 0;
        while done < n {
            // Output samples until the next event, at the current speed.
            let next = seq.events.get(self.cursor).map_or(f64::INFINITY, |e| e.sample as f64);
            let until = ((next - self.position) / self.speed).ceil().max(0.0);
            let chunk = (until as usize).min(n - done);
            if chunk > 0 {
                synth.render(&mut left[done..done + chunk], &mut right[done..done + chunk]);
                done += chunk;
                self.position += chunk as f64 * self.speed;
            }
            while let Some(e) = seq.events.get(self.cursor) {
                if e.sample as f64 > self.position {
                    break;
                }
                self.apply(synth, e.kind);
                self.cursor += 1;
            }
        }
    }

    fn apply(&self, synth: &mut Synthesizer, kind: EventKind) {
        match kind {
            EventKind::NoteOn { channel, key, velocity, track } => {
                // Muted tracks still get their note-offs, so muting mid-note
                // never leaves a key stuck.
                if !self.muted.contains(track) {
                    synth.note_on(channel as i32, key as i32, velocity as i32);
                }
            }
            EventKind::NoteOff { channel, key } => synth.note_off(channel as i32, key as i32),
            EventKind::Controller { channel, number, value } => {
                synth.process_midi_message(channel as i32, 0xB0, number as i32, value as i32)
            }
            EventKind::Program { channel, program } => {
                synth.process_midi_message(channel as i32, 0xC0, program as i32, 0)
            }
            EventKind::PitchBend { channel, value } => {
                let v = value as i32 + 8192;
                synth.process_midi_message(channel as i32, 0xE0, v & 127, v >> 7)
            }
        }
    }
}
