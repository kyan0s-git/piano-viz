use pv_audio::offline::{self, OfflineOptions};
use pv_audio::sequencer::{EventKind, Player, Sequence, SequenceOptions, TrackMask};
use pv_audio::{SoundFont, builtin_font};
use pv_core::{Control, ControlKind, Note, NoteTable, Score, TrackInfo};
use std::sync::Arc;

const SR: u32 = 48_000;

fn font() -> Arc<SoundFont> {
    builtin_font()
}

fn score(notes: Vec<Note>, controls: Vec<Control>) -> Score {
    let tracks = vec![TrackInfo { note_count: notes.len() as u32, ..Default::default() }; 2];
    Score { notes: NoteTable::new(notes), controls, tracks, duration: 4.0, ..Default::default() }
}

fn note(start: f32, duration: f32, pitch: u8, track: u8) -> Note {
    Note { start, duration, sustain: 0.0, pitch, velocity: 100, track, flags: 0 }
}

fn pedal(time: f64, down: bool) -> Control {
    Control {
        time,
        channel: 0,
        kind: ControlKind::Controller { number: 64, value: if down { 127 } else { 0 } },
    }
}

/// Energy of one frequency in a mono signal (Goertzel).
fn power_at(x: &[f32], freq: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / SR as f32;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &v in x {
        let s = v + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    s1 * s1 + s2 * s2 - c * s1 * s2
}

fn left(interleaved: &[f32]) -> Vec<f32> {
    interleaved.iter().step_by(2).copied().collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn render(s: &Score, start: f64, end: f64) -> Vec<f32> {
    offline::render(s, &font(), &OfflineOptions::default(), start, end, |_| {}).unwrap()
}

#[test]
fn builtin_piano_generates_quickly_and_covers_the_keyboard() {
    let t = std::time::Instant::now();
    let f = font();
    eprintln!("built-in piano ready in {:?}", t.elapsed());
    assert_eq!(f.get_presets().len(), 1);
    assert!(f.get_sample_headers().len() >= 28);
}

#[test]
fn a440_sounds_at_440_hz() {
    let s = score(vec![note(0.0, 1.0, 69, 0)], vec![]);
    let audio = left(&render(&s, 0.0, 1.0));
    let sustain = &audio[SR as usize / 10..SR as usize / 2];
    let at = power_at(sustain, 440.0);
    for off in [415.3, 466.2, 220.0 * 1.5] {
        assert!(at > power_at(sustain, off) * 20.0, "440 Hz should dominate {off} Hz");
    }
}

#[test]
fn every_piano_key_sounds_in_tune() {
    // Samples are every six semitones and pitch-shifted between: check the
    // fundamental lands where it should across the range.
    for key in [21u8, 33, 45, 50, 60, 64, 72, 81, 90, 100, 108] {
        let s = score(vec![note(0.0, 0.6, key, 0)], vec![]);
        let audio = left(&render(&s, 0.0, 0.6));
        let x = &audio[SR as usize / 20..SR as usize / 2];
        let f = 440.0 * 2f32.powf((key as f32 - 69.0) / 12.0);
        let semitone_up = f * 2f32.powf(1.0 / 12.0);
        // Low keys radiate weakly at the fundamental on real pianos too;
        // test the 2nd partial there.
        let (test, off) = if key < 33 { (f * 2.0, semitone_up * 2.0) } else { (f, semitone_up) };
        assert!(power_at(x, test) > power_at(x, off) * 4.0, "key {key}");
    }
}

#[test]
fn offline_render_is_deterministic_and_sane() {
    let s = pv_midi::parse(&pv_midi::demo::prelude()).unwrap();
    let a = render(&s, 10.0, 16.0);
    let b = render(&s, 10.0, 16.0);
    assert!(a == b, "same inputs, same samples");
    assert_eq!(a.len(), 6 * SR as usize * 2);
    assert!(a.iter().all(|v| v.is_finite()));
    let peak = a.iter().fold(0f32, |m, v| m.max(v.abs()));
    let level = rms(&a);
    eprintln!("demo 10-16 s: peak {peak:.3}, rms {level:.4}");
    assert!(peak < 1.0, "clipping: peak {peak}");
    assert!(level > 0.07, "too quiet: rms {level}");
}

#[test]
fn seeking_restores_the_pedal() {
    // Pedal goes down at 0 and stays down. A note released at 1.1 s should
    // keep ringing when the render starts at 1.0 s — only if the seek
    // replayed the pedal.
    let s = score(vec![note(1.0, 0.1, 60, 0)], vec![pedal(0.0, true)]);
    let audio = left(&render(&s, 1.0, 2.0));
    let late = &audio[(0.6 * SR as f32) as usize..(0.9 * SR as f32) as usize];
    assert!(rms(late) > 0.005, "pedal should hold the note: rms {}", rms(late));

    let dry = score(vec![note(1.0, 0.1, 60, 0)], vec![]);
    let audio = left(&render(&dry, 1.0, 2.0));
    let late_dry = &audio[(0.6 * SR as f32) as usize..(0.9 * SR as f32) as usize];
    assert!(rms(late_dry) < rms(late) / 4.0, "without pedal it should be damped");
}

#[test]
fn muted_tracks_are_silent_and_muting_never_sticks_notes() {
    let s = score(vec![note(0.0, 0.5, 60, 1)], vec![]);
    let opts =
        OfflineOptions { muted: TrackMask::from_muted(&[false, true]), ..Default::default() };
    let audio = offline::render(&s, &font(), &opts, 0.0, 1.0, |_| {}).unwrap();
    assert!(audio.iter().all(|v| v.abs() < 1e-3), "muted track sounded");

    // Mute mid-note through the player: the note-off must still arrive.
    let seq = Sequence::new(&s, SR, SequenceOptions::default());
    let mut synth =
        rustysynth::Synthesizer::new(&font(), &rustysynth::SynthesizerSettings::new(SR as i32))
            .unwrap();
    let mut player = Player::default();
    let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
    player.render(&seq, &mut synth, &mut l, &mut r); // 0.1 s: note is on
    player.muted = TrackMask::from_muted(&[false, true]);
    for _ in 0..20 {
        player.render(&seq, &mut synth, &mut l, &mut r); // to 2.1 s
    }
    assert!(rms(&l) < 1e-3, "note stuck after muting: rms {}", rms(&l));
}

#[test]
fn same_sample_release_and_restrike_resounds() {
    let s = score(vec![note(0.0, 0.5, 60, 0), note(0.5, 0.5, 60, 0)], vec![]);
    let seq = Sequence::new(&s, SR, SequenceOptions::default());
    let at_half: Vec<_> =
        seq.events.iter().filter(|e| e.sample == SR as u64 / 2).map(|e| e.kind).collect();
    assert!(
        matches!(at_half[..], [EventKind::NoteOff { .. }, EventKind::NoteOn { .. }]),
        "{at_half:?}"
    );
}

#[test]
fn half_speed_takes_twice_as_long() {
    let s = score(vec![note(0.0, 0.1, 60, 0), note(1.0, 0.1, 72, 0)], vec![]);
    let seq = Sequence::new(&s, SR, SequenceOptions::default());
    let mut synth =
        rustysynth::Synthesizer::new(&font(), &rustysynth::SynthesizerSettings::new(SR as i32))
            .unwrap();
    let mut player = Player::default();
    player.speed = 0.5;
    let (mut l, mut r) = (vec![0.0; SR as usize], vec![0.0; SR as usize]);
    player.render(&seq, &mut synth, &mut l, &mut r);
    assert_eq!(player.position(), SR as u64 / 2);
}

#[test]
fn limiter_is_transparent_below_the_knee_and_never_clips() {
    assert_eq!(pv_audio::limit(0.5), 0.5);
    assert_eq!(pv_audio::limit(-0.8), -0.8);
    for x in [0.9f32, 1.5, 10.0, -3.0] {
        let y = pv_audio::limit(x);
        assert!(y.abs() <= 1.0 && y.abs() > 0.8 && y.signum() == x.signum(), "{x} -> {y}");
    }
    assert!(pv_audio::limit(0.9) < pv_audio::limit(1.2), "monotonic");
}

#[test]
fn wav_header_is_well_formed() {
    let mut buf = Vec::new();
    pv_audio::wav::write_f32(&mut buf, &[0.0, 0.5, -0.5, 1.0], 2, SR).unwrap();
    assert_eq!(&buf[..4], b"RIFF");
    assert_eq!(&buf[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize, buf.len() - 8);
    assert_eq!(buf.len(), 44 + 16);
}
