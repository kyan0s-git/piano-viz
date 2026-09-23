use std::ops::Range;

/// Bit flags stored in [`Note::flags`]. The upper nibble holds the channel.
pub mod flags {
    /// Sounding past key release because the sustain pedal was down.
    pub const PEDAL: u8 = 1 << 0;
    /// Pitch is a black key.
    pub const BLACK: u8 = 1 << 1;
    /// No note-off was found; the note was clamped to the end of its track.
    pub const CLAMPED: u8 = 1 << 2;
    /// Channel 10 percussion: audible, but never drawn on the keyboard.
    pub const PERCUSSION: u8 = 1 << 3;
    pub const CHANNEL_SHIFT: u8 = 4;
}

/// One note, packed to 16 bytes so it uploads to the GPU without conversion.
///
/// Layout must match `Note` in the WGSL shaders.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Note {
    /// Seconds from song start.
    pub start: f32,
    /// Seconds the key is held. Always > 0.
    pub duration: f32,
    /// Extra seconds the note keeps sounding after release, from the pedal.
    pub sustain: f32,
    pub pitch: u8,
    pub velocity: u8,
    /// Index into the score's track list, saturating at 255.
    pub track: u8,
    /// See [`flags`].
    pub flags: u8,
}

impl Note {
    #[inline]
    pub fn end(&self) -> f32 {
        self.start + self.duration
    }

    #[inline]
    pub fn channel(&self) -> u8 {
        self.flags >> flags::CHANNEL_SHIFT
    }

    #[inline]
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// All notes of a score, sorted by `(start, pitch)`.
///
/// Sorting by start makes the renderer's visible-window query two binary
/// searches and the sequencer's playback a forward scan.
#[derive(Clone, Debug, Default)]
pub struct NoteTable {
    notes: Vec<Note>,
    /// Longest `duration + sustain` of any note: how far back a note can start
    /// and still reach into the visible window.
    max_extent: f32,
}

impl NoteTable {
    pub fn new(mut notes: Vec<Note>) -> Self {
        // Pack (start, pitch) into one key. Start is non-negative, so its IEEE
        // bits sort the same as its value.
        notes.sort_unstable_by_key(|n| ((n.start.max(0.0).to_bits() as u64) << 8) | n.pitch as u64);
        let max_extent = notes.iter().map(|n| n.duration + n.sustain).fold(0.0, f32::max);
        Self { notes, max_extent }
    }

    #[inline]
    pub fn as_slice(&self) -> &[Note] {
        &self.notes
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    #[inline]
    pub fn max_extent(&self) -> f32 {
        self.max_extent
    }

    /// Index range of every note that overlaps `[t0, t1)`.
    ///
    /// May include a few notes that ended before `t0` — those whose start lies
    /// within `max_extent` of it — which the caller discards cheaply. It never
    /// misses one, which is the property that matters: a long pedal note whose
    /// start is far off-screen still has its body drawn.
    pub fn overlapping(&self, t0: f32, t1: f32) -> Range<usize> {
        let lo = self.notes.partition_point(|n| n.start < t0 - self.max_extent);
        let hi = self.notes.partition_point(|n| n.start < t1);
        lo..hi.max(lo)
    }

    /// Index range of notes whose start lies in `[t0, t1)`.
    pub fn starting_in(&self, t0: f32, t1: f32) -> Range<usize> {
        let lo = self.notes.partition_point(|n| n.start < t0);
        let hi = self.notes.partition_point(|n| n.start < t1);
        lo..hi.max(lo)
    }

    /// Lowest and highest pitch present, ignoring percussion.
    pub fn pitch_range(&self) -> Option<(u8, u8)> {
        let mut it = self.notes.iter().filter(|n| !n.has(flags::PERCUSSION)).map(|n| n.pitch);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f32, duration: f32, pitch: u8) -> Note {
        Note { start, duration, sustain: 0.0, pitch, velocity: 100, track: 0, flags: 0 }
    }

    #[test]
    fn note_is_16_bytes() {
        assert_eq!(std::mem::size_of::<Note>(), 16);
    }

    #[test]
    fn sorts_by_start_then_pitch() {
        let t = NoteTable::new(vec![note(2.0, 1.0, 60), note(1.0, 1.0, 64), note(1.0, 1.0, 60)]);
        let order: Vec<_> = t.as_slice().iter().map(|n| (n.start, n.pitch)).collect();
        assert_eq!(order, [(1.0, 60), (1.0, 64), (2.0, 60)]);
    }

    #[test]
    fn overlapping_includes_long_note_started_offscreen() {
        // A 30-second pedal note starting at 0 must still be found at t=20,
        // among many short notes that would otherwise bound the search.
        let mut notes = vec![note(0.0, 30.0, 40)];
        notes.extend((0..100).map(|i| note(1.0 + i as f32 * 0.2, 0.1, 60)));
        let t = NoteTable::new(notes);
        let r = t.overlapping(20.0, 23.0);
        assert!(t.as_slice()[r].iter().any(|n| n.pitch == 40));
    }

    #[test]
    fn overlapping_excludes_future_notes() {
        let t = NoteTable::new(vec![note(0.0, 1.0, 60), note(10.0, 1.0, 62)]);
        let r = t.overlapping(0.0, 5.0);
        assert!(t.as_slice()[r].iter().all(|n| n.pitch == 60));
    }

    #[test]
    fn empty_table_queries_are_empty() {
        let t = NoteTable::new(vec![]);
        assert!(t.overlapping(0.0, 10.0).is_empty());
        assert_eq!(t.pitch_range(), None);
    }

    #[test]
    fn pitch_range_ignores_percussion() {
        let mut drum = note(0.0, 1.0, 35);
        drum.flags = flags::PERCUSSION;
        let t = NoteTable::new(vec![drum, note(0.0, 1.0, 60), note(0.0, 1.0, 72)]);
        assert_eq!(t.pitch_range(), Some((60, 72)));
    }
}
