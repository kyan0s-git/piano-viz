//! Live mode: play a MIDI keyboard (or the computer keyboard, or the mouse)
//! and the visuals respond. See docs/07-live-mode.md.
//!
//! Sound and picture take separate paths. MIDI input goes from the device
//! thread straight to the audio callback, never waiting for a UI frame;
//! the picture gets its own queue, drained each frame, and is drawn late by
//! the output latency so light and sound arrive together.

use pv_audio::LiveSender;
use pv_core::{Control, ControlKind, Note, NoteTable, Score, TrackInfo, flags, is_black_key};
use std::sync::Arc;
use std::time::Instant;

/// Seconds of past notes kept on screen when not recording.
const KEEP: f32 = 30.0;
const SUSTAIN_PEDAL: u8 = 64;

pub struct Live {
    pub active: bool,
    connection: Option<midir::MidiInputConnection<()>>,
    pub ports: Vec<String>,
    pub port: Option<String>,
    pub status: String,
    rx: Option<rtrb::Consumer<(Instant, [u8; 3])>>,
    pub rec: Recorder,
    /// Octave of the computer keyboard's A key, as a MIDI C.
    pub octave: u8,
    pub velocity: u8,
    /// Keys the computer keyboard or mouse is holding, so they can be released.
    pub typed: [bool; 128],
}

impl Live {
    pub fn new() -> Self {
        let mut l = Self {
            active: false,
            connection: None,
            ports: Vec::new(),
            port: None,
            status: String::new(),
            rx: None,
            rec: Recorder::new(),
            octave: 60,
            velocity: 96,
            typed: [false; 128],
        };
        l.refresh_ports();
        l
    }

    pub fn refresh_ports(&mut self) {
        match midir::MidiInput::new("piano-viz") {
            Ok(mi) => {
                self.ports = mi.ports().iter().filter_map(|p| mi.port_name(p).ok()).collect();
                if self.connection.is_none() {
                    self.status = if self.ports.is_empty() {
                        "No MIDI devices found".into()
                    } else {
                        format!("{} MIDI device(s)", self.ports.len())
                    };
                }
            }
            Err(e) => {
                self.ports.clear();
                self.status = format!("MIDI input unavailable: {e}");
            }
        }
    }

    /// Connect to a device. `audio` is the engine's live queue, owned from
    /// here on by the device callback.
    pub fn connect(&mut self, name: &str, audio: Option<LiveSender>) {
        self.disconnect();
        let result = (|| -> Result<_, String> {
            let mut mi = midir::MidiInput::new("piano-viz").map_err(|e| e.to_string())?;
            mi.ignore(midir::Ignore::All);
            let port = mi
                .ports()
                .into_iter()
                .find(|p| mi.port_name(p).is_ok_and(|n| n == name))
                .ok_or("device is gone")?;
            let (mut tx, rx) = rtrb::RingBuffer::new(1024);
            let mut audio = audio;
            let conn = mi
                .connect(
                    &port,
                    "piano-viz-in",
                    move |_, msg, _| {
                        // Channel messages only; everything else is ignored.
                        if msg.len() < 2 || !matches!(msg[0] & 0xF0, 0x80 | 0x90 | 0xB0) {
                            return;
                        }
                        let m = [msg[0], msg[1], msg.get(2).copied().unwrap_or(0)];
                        if let Some(a) = &mut audio {
                            a.send(m);
                        }
                        let _ = tx.push((Instant::now(), m));
                    },
                    (),
                )
                .map_err(|e| e.to_string())?;
            Ok((conn, rx))
        })();
        match result {
            Ok((conn, rx)) => {
                self.connection = Some(conn);
                self.rx = Some(rx);
                self.port = Some(name.to_owned());
                self.status = format!("Connected: {name}");
                self.active = true;
            }
            Err(e) => self.status = format!("Couldn't connect: {e}"),
        }
    }

    pub fn disconnect(&mut self) {
        if let Some(c) = self.connection.take() {
            c.close();
        }
        self.rx = None;
        self.port = None;
    }

    pub fn connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Pull this frame's device input into the recorder.
    pub fn drain(&mut self) {
        let Some(rx) = &mut self.rx else { return };
        while let Ok((at, m)) = rx.pop() {
            self.rec.midi(self.rec.at(at), m);
        }
    }

    /// Computer-keyboard key for a semitone offset from the current octave:
    /// the "musical typing" layout, white keys on the home row.
    pub fn typing_offset(key: egui::Key) -> Option<u8> {
        use egui::Key::*;
        Some(match key {
            A => 0,
            W => 1,
            S => 2,
            E => 3,
            D => 4,
            F => 5,
            T => 6,
            G => 7,
            Y => 8,
            H => 9,
            U => 10,
            J => 11,
            K => 12,
            O => 13,
            L => 14,
            P => 15,
            Semicolon => 16,
            _ => return None,
        })
    }
}

/// Turns key events into notes: held notes grow each frame, the sustain
/// pedal keeps released notes ringing, and a recording becomes an ordinary
/// score.
pub struct Recorder {
    origin: Instant,
    held: [Option<(f32, u8)>; 128],
    pedal_down: bool,
    /// Notes released under the pedal and still ringing, by index.
    ringing: Vec<usize>,
    done: Vec<Note>,
    controls: Vec<Control>,
    /// Recording start, when recording.
    pub recording: Option<f32>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            held: [None; 128],
            pedal_down: false,
            ringing: Vec::new(),
            done: Vec::new(),
            controls: Vec::new(),
            recording: None,
        }
    }

    pub fn at(&self, i: Instant) -> f32 {
        i.saturating_duration_since(self.origin).as_secs_f32()
    }

    pub fn now(&self) -> f32 {
        self.at(Instant::now())
    }

    pub fn midi(&mut self, t: f32, [status, d1, d2]: [u8; 3]) {
        match status & 0xF0 {
            0x90 if d2 > 0 => self.note_on(t, d1, d2),
            0x80 | 0x90 => self.note_off(t, d1),
            0xB0 if d1 == SUSTAIN_PEDAL => self.pedal(t, d2 >= 64),
            _ => {}
        }
    }

    pub fn note_on(&mut self, t: f32, key: u8, velocity: u8) {
        let k = key as usize & 127;
        if self.held[k].is_some() {
            self.note_off(t, key);
        }
        // A re-strike ends that key's pedal ring.
        self.ringing.retain(|&i| {
            let n = &mut self.done[i];
            if n.pitch == key {
                n.sustain = (t - n.end()).max(0.0);
                false
            } else {
                true
            }
        });
        self.held[k] = Some((t, velocity.max(1)));
    }

    pub fn note_off(&mut self, t: f32, key: u8) {
        let Some((start, velocity)) = self.held[key as usize & 127].take() else { return };
        let mut f = 0;
        if is_black_key(key) {
            f |= flags::BLACK;
        }
        self.done.push(Note {
            start,
            duration: (t - start).max(0.001),
            sustain: 0.0,
            pitch: key,
            velocity,
            track: 0,
            flags: f,
        });
        if self.pedal_down {
            self.ringing.push(self.done.len() - 1);
            self.done.last_mut().unwrap().flags |= flags::PEDAL;
        }
    }

    pub fn pedal(&mut self, t: f32, down: bool) {
        if self.recording.is_some() {
            let kind = ControlKind::Controller {
                number: SUSTAIN_PEDAL,
                value: if down { 127 } else { 0 },
            };
            self.controls.push(Control { time: t as f64, channel: 0, kind });
        }
        if self.pedal_down && !down {
            for i in self.ringing.drain(..) {
                let n = &mut self.done[i];
                n.sustain = (t - n.end()).max(0.0);
            }
        }
        self.pedal_down = down;
    }

    /// Release everything (live mode off, focus lost).
    pub fn release_all(&mut self, t: f32) {
        for k in 0..128u8 {
            self.note_off(t, k);
        }
        self.pedal(t, false);
    }

    /// The notes to draw at `t`: finished ones still on screen, plus held
    /// and ringing ones extended up to `t`.
    pub fn frame(&mut self, t: f32) -> Arc<Score> {
        if self.recording.is_none() {
            // Forget what has scrolled away.
            let cutoff = t - KEEP;
            if self.done.first().is_some_and(|n| n.end() + n.sustain < cutoff)
                && self.ringing.is_empty()
            {
                self.done.retain(|n| n.end() + n.sustain >= cutoff);
            }
        }
        let mut notes: Vec<Note> = self.done.clone();
        for &i in &self.ringing {
            if let Some(n) = notes.get_mut(i) {
                n.sustain = (t - n.end()).max(0.0);
            }
        }
        for (k, h) in self.held.iter().enumerate() {
            if let Some((start, velocity)) = *h {
                notes.push(Note {
                    start,
                    duration: (t - start).max(0.001),
                    sustain: 0.0,
                    pitch: k as u8,
                    velocity,
                    track: 0,
                    flags: if is_black_key(k as u8) { flags::BLACK } else { 0 },
                });
            }
        }
        let count = notes.len() as u32;
        Arc::new(Score {
            notes: NoteTable::new(notes),
            tracks: vec![TrackInfo {
                name: "Live".into(),
                channel: Some(0),
                note_count: count,
                ..Default::default()
            }],
            duration: t as f64,
            ..Default::default()
        })
    }

    pub fn start_recording(&mut self) {
        let t = self.now();
        self.done.clear();
        self.ringing.clear();
        self.controls.clear();
        self.recording = Some(t);
    }

    /// Stop recording and return it as a score starting at zero.
    pub fn stop_recording(&mut self) -> Option<Score> {
        let t0 = self.recording.take()?;
        let t = self.now();
        self.release_all(t);
        let notes: Vec<Note> = self
            .done
            .iter()
            .filter(|n| n.start >= t0)
            .map(|n| Note { start: n.start - t0, ..*n })
            .collect();
        if notes.is_empty() {
            return None;
        }
        let mut controls: Vec<Control> = self
            .controls
            .iter()
            .map(|c| Control { time: (c.time - t0 as f64).max(0.0), ..*c })
            .collect();
        controls.sort_by(|a, b| a.time.total_cmp(&b.time));
        let count = notes.len() as u32;
        let end = notes.iter().map(|n| n.end() + n.sustain).fold(0.0, f32::max);
        let range =
            notes.iter().fold((127u8, 0u8), |(lo, hi), n| (lo.min(n.pitch), hi.max(n.pitch)));
        Some(Score {
            notes: NoteTable::new(notes),
            controls,
            tracks: vec![TrackInfo {
                name: "Live recording".into(),
                channel: Some(0),
                note_count: count,
                pitch_range: Some(range),
                instrument: Some("Acoustic Grand Piano".into()),
                ..Default::default()
            }],
            duration: end as f64 + 0.5,
            title: Some("Live recording".into()),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_note_grows_until_released() {
        let mut r = Recorder::new();
        r.note_on(1.0, 60, 100);
        let s = r.frame(1.5);
        assert_eq!(s.notes.len(), 1);
        assert!((s.notes.as_slice()[0].duration - 0.5).abs() < 1e-6);
        r.note_off(2.0, 60);
        let s = r.frame(3.0);
        assert!((s.notes.as_slice()[0].duration - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pedal_rings_until_lifted_or_restruck() {
        let mut r = Recorder::new();
        r.pedal(0.0, true);
        r.note_on(0.0, 60, 100);
        r.note_off(0.5, 60);
        let s = r.frame(1.5);
        assert!(
            (s.notes.as_slice()[0].sustain - 1.0).abs() < 1e-6,
            "rings while the pedal is down"
        );
        r.pedal(2.0, false);
        let s = r.frame(5.0);
        assert!((s.notes.as_slice()[0].sustain - 1.5).abs() < 1e-6, "frozen at pedal up");
        r.pedal(6.0, true);
        r.note_on(6.0, 62, 90);
        r.note_off(6.2, 62);
        r.note_on(7.0, 62, 90);
        let ringing =
            r.frame(7.5).notes.as_slice().iter().find(|n| n.start == 6.0).unwrap().sustain;
        assert!((ringing - 0.8).abs() < 1e-5, "re-strike cuts the ring: {ringing}");
    }

    #[test]
    fn velocity_zero_note_on_is_a_release() {
        let mut r = Recorder::new();
        r.midi(0.0, [0x90, 60, 100]);
        r.midi(0.4, [0x90, 60, 0]);
        let n = r.frame(1.0).notes.as_slice()[0];
        assert!((n.duration - 0.4).abs() < 1e-6);
    }

    #[test]
    fn recording_becomes_a_score_from_zero_that_round_trips_to_midi() {
        let mut r = Recorder::new();
        r.recording = Some(10.0);
        r.note_on(10.5, 64, 80);
        r.note_off(11.0, 64);
        r.note_on(11.0, 67, 90);
        r.note_off(11.5, 67);
        let s = r.stop_recording().unwrap();
        assert_eq!(s.notes.len(), 2);
        assert!((s.notes.as_slice()[0].start - 0.5).abs() < 1e-6);
        let back = pv_midi::parse(&pv_midi::write::score_to_smf(&s)).unwrap();
        assert_eq!(back.notes.len(), 2);
        assert_eq!(back.notes.as_slice()[1].pitch, 67);
    }

    #[test]
    fn empty_recording_is_none() {
        let mut r = Recorder::new();
        r.start_recording();
        assert!(r.stop_recording().is_none());
    }
}
