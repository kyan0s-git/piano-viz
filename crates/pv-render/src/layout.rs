//! Keyboard geometry, computed once per key range rather than per frame.

use pv_core::{NoteTable, is_black_key};
use pv_design::KeyRange;

/// Horizontal extent of one key, in fractions of the keyboard width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyGeom {
    pub x0: f32,
    pub x1: f32,
    pub black: bool,
    /// Inside the drawn range.
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyLayout {
    pub lo: u8,
    pub hi: u8,
    pub white_count: u32,
    pub keys: [KeyGeom; 128],
}

/// White keys below each pitch class within an octave.
const WHITE_BEFORE: [u32; 12] = [0, 1, 1, 2, 2, 3, 4, 4, 5, 5, 6, 6];

/// Black-key centers, in white-key widths from the octave's C.
///
/// Dividing the seven white keys of an octave into twelve equal slots — one
/// per semitone — gives `(k + 0.5) * 7/12` for semitone `k`. That reproduces
/// a real piano's grouping, where C# and F# sit left of their gap and D# and
/// A# sit right of theirs. Spacing them evenly between neighbors is the most
/// common tell of a hand-rolled keyboard.
fn black_center(semitone: u8) -> f32 {
    (semitone as f32 + 0.5) * 7.0 / 12.0
}

fn white_before(pitch: u8) -> u32 {
    (pitch / 12) as u32 * 7 + WHITE_BEFORE[(pitch % 12) as usize]
}

impl KeyLayout {
    /// Layout for `lo..=hi`, widened so both ends are white keys.
    pub fn new(lo: u8, hi: u8, black_width_ratio: f32) -> Self {
        let (mut lo, mut hi) = (lo.min(hi), hi.max(lo).min(127));
        while is_black_key(lo) && lo > 0 {
            lo -= 1;
        }
        while is_black_key(hi) && hi < 127 {
            hi += 1;
        }
        let base = white_before(lo) as f32;
        let white_count = white_before(hi) + 1 - white_before(lo);
        let w = white_count as f32;
        let mut keys = [KeyGeom::default(); 128];
        for p in 0..=127u8 {
            let (x0, x1) = if is_black_key(p) {
                let c = (p / 12) as f32 * 7.0 + black_center(p % 12) - base;
                (c - black_width_ratio / 2.0, c + black_width_ratio / 2.0)
            } else {
                let x = white_before(p) as f32 - base;
                (x, x + 1.0)
            };
            keys[p as usize] = KeyGeom {
                x0: x0 / w,
                x1: x1 / w,
                black: is_black_key(p),
                visible: (lo..=hi).contains(&p),
            };
        }
        Self { lo, hi, white_count, keys }
    }

    /// Resolve a Design's key range against the music.
    pub fn for_range(range: KeyRange, notes: &NoteTable, black_width_ratio: f32) -> Self {
        let (lo, hi) = match range {
            KeyRange::Fixed(lo, hi) => (lo, hi),
            KeyRange::Auto => auto_range(notes),
        };
        Self::new(lo, hi, black_width_ratio)
    }
}

/// The music's range with a little margin, at least three octaves wide so a
/// narrow piece doesn't produce comically wide keys.
fn auto_range(notes: &NoteTable) -> (u8, u8) {
    let Some((lo, hi)) = notes.pitch_range() else { return (21, 108) };
    let (mut lo, mut hi) = (lo.saturating_sub(2) as i32, (hi as i32 + 2).min(127));
    while hi - lo < 36 {
        lo = (lo - 1).max(0);
        hi = (hi + 1).min(127);
        if lo == 0 && hi == 127 {
            break;
        }
    }
    (lo as u8, hi as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pv_core::Note;

    #[test]
    fn full_piano_has_52_white_keys() {
        let l = KeyLayout::new(21, 108, 0.58);
        assert_eq!(l.white_count, 52);
        assert!((l.keys[21].x0).abs() < 1e-6 && (l.keys[108].x1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn black_keys_straddle_their_neighbors() {
        let l = KeyLayout::new(21, 108, 0.58);
        for p in 22..108u8 {
            if is_black_key(p) {
                let (k, below, above) =
                    (l.keys[p as usize], l.keys[p as usize - 1], l.keys[p as usize + 1]);
                assert!(k.x0 > below.x0 && k.x0 < below.x1, "{p}");
                assert!(k.x1 > above.x0 && k.x1 < above.x1, "{p}");
            }
        }
    }

    #[test]
    fn c_sharp_sits_left_of_the_gap_and_d_sharp_right() {
        let l = KeyLayout::new(60, 72, 0.58);
        let gap_cd = l.keys[60].x1;
        let gap_de = l.keys[62].x1;
        let mid = |k: KeyGeom| (k.x0 + k.x1) / 2.0;
        assert!(mid(l.keys[61]) < gap_cd);
        assert!(mid(l.keys[63]) > gap_de);
    }

    #[test]
    fn ends_snap_to_white_keys() {
        let l = KeyLayout::new(61, 70, 0.58);
        assert_eq!((l.lo, l.hi), (60, 71));
    }

    #[test]
    fn auto_range_is_at_least_three_octaves() {
        let n = |p| Note {
            start: 0.0,
            duration: 1.0,
            sustain: 0.0,
            pitch: p,
            velocity: 90,
            track: 0,
            flags: 0,
        };
        let l = KeyLayout::for_range(KeyRange::Auto, &NoteTable::new(vec![n(60), n(64)]), 0.58);
        assert!(l.hi - l.lo >= 36);
        let empty = KeyLayout::for_range(KeyRange::Auto, &NoteTable::default(), 0.58);
        assert_eq!((empty.lo, empty.hi), (21, 108));
    }
}
