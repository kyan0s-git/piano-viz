//! The demo piece loaded on first run: the first nineteen bars of Bach's
//! Prelude in C major, BWV 846 (public domain), ending on a rolled C chord.
//!
//! Generated rather than shipped as a file: it's a few hundred bytes of code,
//! and the regular figuration makes a good showcase for falling notes.

use crate::write::{self, Ending, Event};

/// Five-note voicing of each bar, low to high.
const BARS: [[u8; 5]; 19] = [
    [60, 64, 67, 72, 76], // C
    [60, 62, 69, 74, 77], // Dm7/C
    [59, 62, 67, 74, 77], // G7/B
    [60, 64, 67, 72, 76], // C
    [60, 64, 69, 76, 81], // Am/C
    [60, 62, 66, 69, 74], // D7/C
    [59, 62, 67, 74, 79], // G/B
    [59, 60, 64, 67, 72], // Cmaj7/B
    [57, 60, 64, 67, 72], // Am7
    [50, 57, 62, 66, 72], // D7
    [55, 59, 62, 67, 71], // G
    [55, 58, 64, 67, 73], // C#dim7/G
    [53, 57, 62, 69, 74], // Dm/F
    [53, 56, 62, 65, 71], // Bdim7/F
    [52, 55, 60, 67, 72], // C/E
    [52, 53, 57, 60, 65], // Fmaj7/E
    [50, 53, 57, 60, 65], // Dm7
    [43, 50, 55, 59, 65], // G7
    [48, 52, 55, 60, 64], // C
];

const FINAL_CHORD: [u8; 9] = [36, 43, 48, 52, 55, 60, 64, 67, 72];

/// SMF bytes for the demo: two tracks (left hand, right hand) sharing one
/// channel, as piano MIDI usually does, so the pedal sustains both.
pub fn prelude() -> Vec<u8> {
    const TPQN: u16 = 480;
    const S: u64 = TPQN as u64 / 4; // one sixteenth
    let mut meta: Vec<Event> = vec![
        (0, write::track_name("Prelude in C major, BWV 846")),
        (0, write::meta_text(0x02, "J. S. Bach (public domain)")),
        (0, write::tempo(870_000)), // ~69 BPM
        (0, write::time_signature(4, 2)),
    ];
    let mut left: Vec<Event> = vec![(0, write::track_name("Left Hand"))];
    let mut right: Vec<Event> = vec![(0, write::track_name("Right Hand"))];

    // Deterministic humanization: a small hash-driven velocity wobble.
    let wobble = |i: u64| ((i.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 59) as i32 - 16) / 4;

    let mut i = 0u64;
    for (bar, v) in BARS.iter().enumerate() {
        for half in 0..2u64 {
            let t0 = (bar as u64 * 16 + half * 8) * S;
            // Left hand: two held voices.
            left.push((t0, write::note_on(0, v[0], (62 + wobble(i)) as u8)));
            left.push((t0 + 8 * S - 4, write::note_off(0, v[0])));
            left.push((t0 + S, write::note_on(0, v[1], (56 + wobble(i + 1)) as u8)));
            left.push((t0 + 8 * S - 4, write::note_off(0, v[1])));
            // Right hand: the broken chord, twice.
            for (k, &p) in [v[2], v[3], v[4], v[2], v[3], v[4]].iter().enumerate() {
                let t = t0 + (2 + k as u64) * S;
                let accent = if k % 3 == 0 { 6 } else { 0 };
                right.push((
                    t,
                    write::note_on(0, p, (64 + accent + wobble(i + 2 + k as u64)) as u8),
                ));
                right.push((t + S - 6, write::note_off(0, p)));
            }
            // Pedal changes with each harmony: up just before, down just after.
            left.push((t0 + 8 * S - 12, write::controller(0, 64, 0)));
            left.push((t0 + 10, write::controller(0, 64, 127)));
            i += 8;
        }
    }

    // A slow rolled final chord, held.
    let t_end = BARS.len() as u64 * 16 * S;
    for (k, &p) in FINAL_CHORD.iter().enumerate() {
        let track = if p < 60 { &mut left } else { &mut right };
        let t = t_end + k as u64 * 40;
        track.push((t, write::note_on(0, p, (58 + k as i32 * 2) as u8)));
        track.push((t_end + 16 * S, write::note_off(0, p)));
    }
    left.push((t_end + 10, write::controller(0, 64, 127)));
    left.push((t_end + 20 * S, write::controller(0, 64, 0)));
    // Keep the final chord ringing past the release for the pedal tail.
    meta.push((t_end + 20 * S, write::meta_text(0x01, "fine")));

    write::smf(1, TPQN, &[meta, left, right], Ending::EndOfTrack)
}
