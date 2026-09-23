//! One test per row of the edge-case table in docs/03-midi.md, plus the
//! properties the rest of the app relies on.

use pv_core::{ControlKind, flags};
use pv_midi::write::{self, Ending, Event, controller, note_off, note_on, smf, tempo, track_name};

const TPQN: u16 = 480;
const Q: u64 = TPQN as u64; // one quarter = 0.5 s at the default 120 BPM

fn parse(bytes: &[u8]) -> pv_core::Score {
    pv_midi::parse(bytes).expect("should parse")
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn format0_splits_channels_into_tracks() {
    let track = vec![
        (0, note_on(0, 60, 90)),
        (Q, note_off(0, 60)),
        (0, note_on(3, 40, 90)),
        (Q, note_off(3, 40)),
    ];
    let s = parse(&smf(0, TPQN, &[track], Ending::EndOfTrack));
    assert_eq!(s.tracks.len(), 2);
    let tracks: Vec<_> = s.notes.as_slice().iter().map(|n| (n.pitch, n.track)).collect();
    assert!(tracks.contains(&(60, 0)) && tracks.contains(&(40, 1)), "{tracks:?}");
}

#[test]
fn format1_tracks_as_authored_with_tempo_from_track0() {
    let conductor = vec![(0, track_name("Song")), (0, tempo(1_000_000))]; // 60 BPM
    let piano = vec![(0, track_name("Piano")), (Q, note_on(0, 60, 90)), (2 * Q, note_off(0, 60))];
    let s = parse(&smf(1, TPQN, &[conductor, piano], Ending::EndOfTrack));
    assert_eq!(s.title.as_deref(), Some("Song"));
    assert_eq!(s.tracks[1].name, "Piano");
    let n = s.notes.as_slice()[0];
    assert!(close(n.start, 1.0) && close(n.duration, 1.0), "{n:?}");
    assert_eq!(n.track, 1);
}

#[test]
fn format2_sequences_play_back_to_back() {
    let a = vec![(0, note_on(0, 60, 90)), (Q, note_off(0, 60))];
    let b = vec![(0, note_on(0, 62, 90)), (Q, note_off(0, 62))];
    let s = parse(&smf(2, TPQN, &[a, b], Ending::EndOfTrack));
    let second = s.notes.as_slice().iter().find(|n| n.pitch == 62).unwrap();
    assert!(close(second.start, 0.5), "{second:?}");
    assert!(s.warnings.iter().any(|w| w.contains("Format 2")));
}

#[test]
fn note_on_velocity_zero_is_note_off() {
    let t = vec![(0, note_on(0, 60, 90)), (Q, note_on(0, 60, 0))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    assert_eq!(s.notes.len(), 1);
    assert!(close(s.notes.as_slice()[0].duration, 0.5));
}

#[test]
fn retrigger_closes_previous_note() {
    let t = vec![(0, note_on(0, 60, 90)), (Q, note_on(0, 60, 80)), (2 * Q, note_off(0, 60))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    let n = s.notes.as_slice();
    assert_eq!(n.len(), 2);
    assert!(close(n[0].duration, 0.5) && close(n[1].start, 0.5) && n[1].velocity == 80);
}

#[test]
fn orphan_note_off_is_discarded() {
    let t = vec![(0, note_off(0, 61)), (0, note_on(0, 60, 90)), (Q, note_off(0, 60))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    assert_eq!(s.notes.len(), 1);
    assert!(s.warnings.iter().any(|w| w.contains("without a matching note-on")));
}

#[test]
fn missing_note_off_clamps_to_track_end() {
    let t = vec![(0, note_on(0, 60, 90)), (4 * Q, controller(0, 7, 100))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    let n = s.notes.as_slice()[0];
    assert!(close(n.duration, 2.0), "{n:?}");
    assert!(n.has(flags::CLAMPED));
    assert!(s.warnings.iter().any(|w| w.contains("no note-off")));
}

#[test]
fn zero_duration_note_gets_a_floor() {
    let t = vec![(Q, note_on(0, 60, 90)), (Q, note_off(0, 60))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    assert!(s.notes.as_slice()[0].duration >= 0.001);
}

#[test]
fn sustain_pedal_extends_to_release() {
    let t = vec![
        (0, controller(0, 64, 127)),
        (0, note_on(0, 60, 90)),
        (Q, note_off(0, 60)),
        (4 * Q, controller(0, 64, 0)),
    ];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    let n = s.notes.as_slice()[0];
    assert!(n.has(flags::PEDAL));
    assert!(close(n.duration, 0.5) && close(n.sustain, 1.5), "{n:?}");
}

#[test]
fn sustain_is_cut_by_the_next_strike_of_the_same_key() {
    let t = vec![
        (0, controller(0, 64, 127)),
        (0, note_on(0, 60, 90)),
        (Q, note_off(0, 60)),
        (2 * Q, note_on(0, 60, 90)),
        (3 * Q, note_off(0, 60)),
        (8 * Q, controller(0, 64, 0)),
    ];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    let first = s.notes.as_slice()[0];
    assert!(close(first.sustain, 0.5), "{first:?}");
}

#[test]
fn pedal_on_other_channel_does_not_sustain() {
    let t = vec![(0, controller(1, 64, 127)), (0, note_on(0, 60, 90)), (Q, note_off(0, 60))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    assert_eq!(s.notes.as_slice()[0].sustain, 0.0);
}

#[test]
fn note_released_before_pedal_down_is_not_sustained() {
    let t = vec![(0, note_on(0, 60, 90)), (Q, note_off(0, 60)), (2 * Q, controller(0, 64, 127))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    assert!(!s.notes.as_slice()[0].has(flags::PEDAL));
}

#[test]
fn channel_10_is_percussion() {
    let t = vec![
        (0, note_on(9, 36, 100)),
        (Q, note_off(9, 36)),
        (0, note_on(0, 60, 90)),
        (Q, note_off(0, 60)),
    ];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    let drum = s.notes.as_slice().iter().find(|n| n.pitch == 36).unwrap();
    assert!(drum.has(flags::PERCUSSION));
    assert_eq!(s.notes.pitch_range(), Some((60, 60)));
}

#[test]
fn pitch_bend_and_program_are_kept_for_the_synth() {
    let t =
        vec![(0, write::program(0, 4)), (Q, write::pitch_bend(0, -8192)), (Q, note_on(0, 60, 90))];
    let s = parse(&smf(1, TPQN, &[t], Ending::EndOfTrack));
    assert!(s.controls.iter().any(|c| c.kind == ControlKind::Program(4)));
    assert!(s.controls.iter().any(|c| c.kind == ControlKind::PitchBend(-8192)));
    assert_eq!(s.tracks[0].instrument.as_deref(), Some("Electric Piano 1"));
}

#[test]
fn empty_tracks_are_kept_for_their_names() {
    let names = vec![(0, track_name("Notes"))];
    let empty = vec![(0, track_name("Lyrics"))];
    let s = parse(&smf(1, TPQN, &[names, empty], Ending::EndOfTrack));
    assert_eq!(s.tracks.len(), 2);
    assert_eq!(s.visible_tracks().count(), 0);
}

#[test]
fn file_with_no_notes_loads() {
    let s = parse(&smf(1, TPQN, &[vec![]], Ending::EndOfTrack));
    assert!(s.notes.is_empty());
}

#[test]
fn missing_end_of_track_warns_but_loads() {
    let t = vec![(0, note_on(0, 60, 90)), (Q, note_off(0, 60))];
    let s = parse(&smf(1, TPQN, &[t], Ending::None));
    assert_eq!(s.notes.len(), 1);
    assert!(s.warnings.iter().any(|w| w.contains("truncated")));
}

#[test]
fn truncated_file_keeps_what_parsed() {
    let t: Vec<Event> = (0..50)
        .flat_map(|i| [(i * Q, note_on(0, 60, 90)), (i * Q + Q / 2, note_off(0, 60))])
        .collect();
    let bytes = smf(1, TPQN, &[t], Ending::EndOfTrack);
    let cut = &bytes[..bytes.len() * 2 / 3];
    let s = parse(cut);
    assert!(s.notes.len() > 10, "kept {}", s.notes.len());
}

#[test]
fn smpte_division_ignores_tempo() {
    // Division 0xE728: -25 fps, 40 ticks per frame => 1000 ticks per second.
    let t = vec![(0, tempo(250_000)), (1000, note_on(0, 60, 90)), (2000, note_off(0, 60))];
    let s = parse(&smf(1, 0xE728, &[t], Ending::EndOfTrack));
    let n = s.notes.as_slice()[0];
    assert!(close(n.start, 1.0) && close(n.duration, 1.0), "{n:?}");
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    for bytes in [&b""[..], b"hello world", b"MThd\x00\x00\x00\x06", &[0xFF; 64]] {
        assert!(pv_midi::parse(bytes).is_err());
    }
}

#[test]
fn latin1_track_names_decode() {
    let mut name = vec![0xFF, 0x03, 4];
    name.extend_from_slice(&[b'C', b'a', 0xE9, b'!']); // "Caé!" in Latin-1
    let s = parse(&smf(1, TPQN, &[vec![(0, name)]], Ending::EndOfTrack));
    assert_eq!(s.tracks[0].name, "Caé!");
}

#[test]
fn demo_piece_parses_with_two_hands_and_pedal() {
    let s = parse(&pv_midi::demo::prelude());
    let names: Vec<_> = s.visible_tracks().map(|(_, t)| t.name.as_str()).collect();
    assert_eq!(names, ["Left Hand", "Right Hand"]);
    // 19 bars x 2 halves x 8 notes + the final chord.
    assert_eq!(s.notes.len(), 19 * 2 * 8 + 9);
    assert!(s.notes.as_slice().iter().any(|n| n.has(flags::PEDAL)));
    assert!(s.duration > 60.0 && s.duration < 80.0, "{}", s.duration);
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
}

#[test]
fn score_round_trips_through_smf() {
    let original = parse(&pv_midi::demo::prelude());
    let again = parse(&write::score_to_smf(&original));
    assert_eq!(again.notes.len(), original.notes.len());
    for (a, b) in original.notes.as_slice().iter().zip(again.notes.as_slice()) {
        assert_eq!((a.pitch, a.velocity), (b.pitch, b.velocity));
        // 960 ticks per quarter at 120 BPM: ~0.5 ms resolution.
        assert!((a.start - b.start).abs() < 1e-3 && (a.duration - b.duration).abs() < 1e-3);
    }
}

/// Load budget from docs/11-performance.md: 500k notes in under a second.
/// Run with `cargo test --release -p pv-midi -- --ignored`.
#[test]
#[ignore = "timing test; meaningful only in release builds"]
fn half_million_notes_parse_under_a_second() {
    let tracks: Vec<Vec<Event>> = (0..16u64)
        .map(|tr| {
            (0..31_250u64)
                .flat_map(|i| {
                    let p = 21 + ((i * 7 + tr * 5) % 88) as u8;
                    let t = i * 24 + tr;
                    [(t, note_on(0, p, 80)), (t + 20, note_off(0, p))]
                })
                .collect()
        })
        .collect();
    let bytes = smf(1, TPQN, &tracks, Ending::EndOfTrack);
    let t = std::time::Instant::now();
    let s = parse(&bytes);
    let took = t.elapsed();
    eprintln!("parsed {} notes in {took:?}", s.notes.len());
    assert_eq!(s.notes.len(), 500_000);
    assert!(took.as_secs_f64() < 1.0, "{took:?}");
}
