//! A minimal Standard MIDI File writer.
//!
//! Used to save live recordings as `.mid`, to build the demo piece, and by
//! tests to construct exactly the malformed files real software produces.

use pv_core::{ControlKind, Score};

/// One timed event: absolute tick plus the raw bytes after the delta.
pub type Event = (u64, Vec<u8>);

pub fn note_on(ch: u8, pitch: u8, vel: u8) -> Vec<u8> {
    vec![0x90 | (ch & 15), pitch & 127, vel & 127]
}

pub fn note_off(ch: u8, pitch: u8) -> Vec<u8> {
    vec![0x80 | (ch & 15), pitch & 127, 0]
}

pub fn controller(ch: u8, number: u8, value: u8) -> Vec<u8> {
    vec![0xB0 | (ch & 15), number & 127, value & 127]
}

pub fn program(ch: u8, program: u8) -> Vec<u8> {
    vec![0xC0 | (ch & 15), program & 127]
}

pub fn pitch_bend(ch: u8, bend: i16) -> Vec<u8> {
    let v = (bend.clamp(-8192, 8191) + 8192) as u16;
    vec![0xE0 | (ch & 15), (v & 127) as u8, (v >> 7) as u8]
}

pub fn tempo(us_per_quarter: u32) -> Vec<u8> {
    let b = us_per_quarter.to_be_bytes();
    vec![0xFF, 0x51, 3, b[1], b[2], b[3]]
}

pub fn time_signature(num: u8, den_pow: u8) -> Vec<u8> {
    vec![0xFF, 0x58, 4, num, den_pow, 24, 8]
}

pub fn track_name(name: &str) -> Vec<u8> {
    meta_text(0x03, name)
}

pub fn meta_text(kind: u8, text: &str) -> Vec<u8> {
    let mut v = vec![0xFF, kind];
    write_vlq(&mut v, text.len() as u32);
    v.extend_from_slice(text.as_bytes());
    v
}

/// How a track should end. Tests need the malformed variants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    EndOfTrack,
    /// Omit the End of Track meta event, as some broken exporters do.
    None,
}

/// Serialize tracks of `(tick, bytes)` events. Events are stably sorted by
/// tick, so same-tick events keep the order given.
pub fn smf(format: u16, division: u16, tracks: &[Vec<Event>], ending: Ending) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
    out.extend_from_slice(&division.to_be_bytes());

    for events in tracks {
        let mut sorted: Vec<&Event> = events.iter().collect();
        sorted.sort_by_key(|e| e.0);
        let mut body = Vec::new();
        let mut last = 0;
        for (tick, bytes) in sorted {
            write_vlq(&mut body, (tick - last) as u32);
            body.extend_from_slice(bytes);
            last = *tick;
        }
        if ending == Ending::EndOfTrack {
            body.extend_from_slice(&[0, 0xFF, 0x2F, 0]);
        }
        out.extend_from_slice(b"MTrk");
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
    }
    out
}

fn write_vlq(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 5];
    let mut i = buf.len() - 1;
    buf[i] = (v & 0x7F) as u8;
    v >>= 7;
    while v > 0 {
        i -= 1;
        buf[i] = (v & 0x7F) as u8 | 0x80;
        v >>= 7;
    }
    out.extend_from_slice(&buf[i..]);
}

/// Serialize a score (for example a live recording) as a format 1 file at a
/// fixed 120 BPM, one MIDI track per score track.
pub fn score_to_smf(score: &Score) -> Vec<u8> {
    const TPQN: u16 = 960;
    // 120 BPM: two quarters per second.
    let tick = |s: f64| (s.max(0.0) * TPQN as f64 * 2.0).round() as u64;
    let n_tracks = score.tracks.len().max(1);
    let mut tracks: Vec<Vec<Event>> = vec![Vec::new(); n_tracks + 1];
    tracks[0].push((0, tempo(500_000)));
    if let Some(t) = &score.title {
        tracks[0].push((0, track_name(t)));
    }
    for (i, info) in score.tracks.iter().enumerate() {
        tracks[i + 1].push((0, track_name(&info.label(i))));
    }
    for n in score.notes.as_slice() {
        let t = &mut tracks[(n.track as usize).min(n_tracks - 1) + 1];
        let ch = n.channel();
        t.push((tick(n.start as f64), note_on(ch, n.pitch, n.velocity.max(1))));
        t.push((tick(n.end() as f64), note_off(ch, n.pitch)));
    }
    // Controls go on the first note track; which track carries a channel
    // message doesn't matter to playback.
    for c in &score.controls {
        let bytes = match c.kind {
            ControlKind::Controller { number, value } => controller(c.channel, number, value),
            ControlKind::Program(p) => program(c.channel, p),
            ControlKind::PitchBend(b) => pitch_bend(c.channel, b),
        };
        tracks[1.min(n_tracks)].push((tick(c.time), bytes));
    }
    // Same tick: note-offs before note-ons so a re-struck key isn't cut.
    for t in &mut tracks {
        t.sort_by_key(|(tick, b)| (*tick, b[0] & 0xF0 != 0x80));
    }
    smf(1, TPQN, &tracks, Ending::EndOfTrack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_matches_spec_examples() {
        for (v, want) in [
            (0u32, &[0x00][..]),
            (0x7F, &[0x7F]),
            (0x80, &[0x81, 0x00]),
            (0x2000, &[0xC0, 0x00]),
            (0x0FFF_FFFF, &[0xFF, 0xFF, 0xFF, 0x7F]),
        ] {
            let mut out = Vec::new();
            write_vlq(&mut out, v);
            assert_eq!(out, want, "{v:#x}");
        }
    }
}
