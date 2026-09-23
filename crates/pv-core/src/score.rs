use crate::{NoteTable, TempoMap, flags};

/// A loaded piece of music. Immutable after load, so the renderer and the
/// audio thread share it behind an `Arc` with no synchronization.
#[derive(Clone, Debug, Default)]
pub struct Score {
    pub notes: NoteTable,
    /// Non-note events the synth needs, sorted by time.
    pub controls: Vec<Control>,
    pub tempo: TempoMap,
    pub tracks: Vec<TrackInfo>,
    /// Seconds from start to the last event.
    pub duration: f64,
    pub title: Option<String>,
    pub copyright: Option<String>,
    /// Problems found and worked around during load, for display.
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub time: f64,
    pub channel: u8,
    pub kind: ControlKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Controller {
        number: u8,
        value: u8,
    },
    Program(u8),
    /// Centered at 0, range -8192..=8191.
    PitchBend(i16),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackInfo {
    pub name: String,
    pub instrument: Option<String>,
    pub program: Option<u8>,
    pub channel: Option<u8>,
    pub note_count: u32,
    pub pitch_range: Option<(u8, u8)>,
}

impl TrackInfo {
    /// Name for display: the track name, else the instrument, else a number.
    pub fn label(&self, index: usize) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_owned();
        }
        if let Some(i) = &self.instrument {
            return i.trim().to_owned();
        }
        format!("Track {}", index + 1)
    }
}

impl Score {
    /// Tracks that actually contain drawable notes.
    pub fn visible_tracks(&self) -> impl Iterator<Item = (usize, &TrackInfo)> {
        self.tracks.iter().enumerate().filter(|(_, t)| t.note_count > 0 && t.channel != Some(9))
    }

    /// A pitch that separates left hand from right, for hand-based coloring.
    ///
    /// Picks the least-used pitch near middle C, favoring C4 on ties, so the
    /// split falls in the gap between the hands rather than through a busy
    /// register.
    pub fn suggested_split(&self) -> u8 {
        let mut hist = [0u32; 128];
        for n in self.notes.as_slice().iter().filter(|n| !n.has(flags::PERCUSSION)) {
            hist[n.pitch as usize] += 1;
        }
        (48u8..=72)
            .min_by_key(|&p| {
                let busy: u32 = (p - 2..=p + 2).map(|q| hist[q as usize]).sum();
                (busy, (p as i32 - 60).unsigned_abs())
            })
            .unwrap_or(60)
    }

    /// Seconds before the first note. Large values suggest trimming lead-in.
    pub fn lead_in(&self) -> f64 {
        self.notes.as_slice().first().map_or(0.0, |n| n.start as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Note;

    fn n(pitch: u8) -> Note {
        Note { start: 0.0, duration: 1.0, sustain: 0.0, pitch, velocity: 90, track: 0, flags: 0 }
    }

    #[test]
    fn split_lands_in_gap_between_hands() {
        let mut notes: Vec<_> = (36..50).map(n).collect();
        notes.extend((64..80).map(n));
        let score = Score { notes: NoteTable::new(notes), ..Default::default() };
        let split = score.suggested_split();
        assert!((52..=62).contains(&split), "split {split}");
    }

    #[test]
    fn split_defaults_to_middle_c_when_empty() {
        assert_eq!(Score::default().suggested_split(), 60);
    }
}
