use crate::MidiError;
use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use pv_core::{
    Control, ControlKind, Note, NoteTable, Score, TempoMap, TimeSignature, TrackInfo, flags, gm,
    is_black_key,
};

/// Shortest note we'll produce. Zero-length notes would be invisible and
/// inaudible, but the file clearly meant *something* to sound.
const MIN_DURATION: f32 = 0.001;
const PERCUSSION_CHANNEL: u8 = 9;
const SUSTAIN_PEDAL: u8 = 64;

/// A note while still in ticks, before the tempo map is applied.
struct RawNote {
    start: u64,
    end: u64,
    pitch: u8,
    velocity: u8,
    channel: u8,
    track: usize,
    clamped: bool,
}

pub fn parse(bytes: &[u8]) -> Result<Score, MidiError> {
    let smf = Smf::parse(bytes).map_err(|e| MidiError::Invalid(e.to_string()))?;
    let mut warnings = Vec::new();

    // Format 2 tracks are independent sequences played one after another.
    // Offsetting each by the length of those before it lets one tempo map and
    // one timeline cover them all.
    let offsets: Vec<u64> = if smf.header.format == Format::Sequential && smf.tracks.len() > 1 {
        warnings.push("Format 2 file: independent sequences were played back to back".into());
        let mut acc = 0;
        smf.tracks
            .iter()
            .map(|t| {
                let start = acc;
                acc += t.iter().map(|e| e.delta.as_int() as u64).sum::<u64>();
                start
            })
            .collect()
    } else {
        vec![0; smf.tracks.len()]
    };

    // Pass 1: timing. Tempo applies globally, wherever it appears.
    let mut tempo_changes = Vec::new();
    let mut sig_ticks = Vec::new();
    for (track, offset) in smf.tracks.iter().zip(&offsets) {
        let mut tick = *offset;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) => {
                    tempo_changes.push((tick, us.as_int()))
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_pow, _, _)) => {
                    sig_ticks.push((tick, num, den_pow))
                }
                _ => {}
            }
        }
    }
    let mut tempo = match smf.header.timing {
        Timing::Metrical(tpqn) => TempoMap::metrical(tpqn.as_int(), tempo_changes),
        Timing::Timecode(fps, sub) => TempoMap::timecode(fps.as_f32() as f64, sub),
    };
    tempo.set_time_signatures(
        sig_ticks
            .into_iter()
            .map(|(tick, numerator, den_pow)| TimeSignature {
                seconds: tempo.seconds(tick),
                numerator: numerator.max(1),
                denominator: 1u8.checked_shl(den_pow as u32).unwrap_or(4),
            })
            .collect(),
    );

    // Pass 2: notes and controls.
    let single_track = smf.header.format == Format::SingleTrack;
    let mut raw = Vec::new();
    let mut controls = Vec::new();
    let mut tracks: Vec<TrackInfo> = Vec::new();
    let mut last_tick = 0u64;
    let mut orphan_offs = 0usize;
    let mut unterminated = 0usize;
    let mut title = None;
    let mut copyright = None;

    // Format 0 packs every part into one track; split it by channel so each
    // part gets its own color and mute. Tracks are allocated lazily.
    let mut channel_track = [usize::MAX; 16];

    for (ti, (track, offset)) in smf.tracks.iter().zip(&offsets).enumerate() {
        if !single_track {
            tracks.push(TrackInfo::default());
        }
        let mut open: [[Option<(u64, u8, usize)>; 128]; 16] = [[None; 128]; 16];
        let mut tick = *offset;
        let mut ended = false;

        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    let tr = if single_track {
                        if channel_track[ch as usize] == usize::MAX {
                            channel_track[ch as usize] = tracks.len();
                            tracks.push(TrackInfo {
                                name: format!("Channel {}", ch + 1),
                                ..Default::default()
                            });
                        }
                        channel_track[ch as usize]
                    } else {
                        ti
                    };
                    let info = &mut tracks[tr];
                    info.channel.get_or_insert(ch);
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            let k = key.as_int() as usize;
                            // Re-striking a sounding key ends the previous note
                            // there, as a real piano would.
                            if let Some((s, v, t)) = open[ch as usize][k].take() {
                                raw.push(RawNote {
                                    start: s,
                                    end: tick,
                                    pitch: k as u8,
                                    velocity: v,
                                    channel: ch,
                                    track: t,
                                    clamped: false,
                                });
                            }
                            open[ch as usize][k] = Some((tick, vel.as_int(), tr));
                        }
                        // Note-on at velocity 0 is a note-off: very common.
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            let k = key.as_int() as usize;
                            match open[ch as usize][k].take() {
                                Some((s, v, t)) => raw.push(RawNote {
                                    start: s,
                                    end: tick,
                                    pitch: k as u8,
                                    velocity: v,
                                    channel: ch,
                                    track: t,
                                    clamped: false,
                                }),
                                None => orphan_offs += 1,
                            }
                        }
                        MidiMessage::Controller { controller, value } => controls.push((
                            tick,
                            ch,
                            ControlKind::Controller {
                                number: controller.as_int(),
                                value: value.as_int(),
                            },
                        )),
                        MidiMessage::ProgramChange { program } => {
                            let p = program.as_int();
                            info.program.get_or_insert(p);
                            controls.push((tick, ch, ControlKind::Program(p)));
                        }
                        MidiMessage::PitchBend { bend } => {
                            controls.push((tick, ch, ControlKind::PitchBend(bend.as_int())))
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(meta) => {
                    let info = if single_track { None } else { tracks.get_mut(ti) };
                    match meta {
                        MetaMessage::TrackName(b) => {
                            let name = decode_text(b);
                            if ti == 0 && title.is_none() && !name.trim().is_empty() {
                                title = Some(name.trim().to_owned());
                            }
                            if let Some(i) = info {
                                i.name = name;
                            }
                        }
                        MetaMessage::InstrumentName(b) => {
                            if let Some(i) = info {
                                i.instrument = Some(decode_text(b));
                            }
                        }
                        MetaMessage::Copyright(b) => copyright = Some(decode_text(b)),
                        MetaMessage::EndOfTrack => ended = true,
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Anything still held at the end of the track never got a note-off.
        for (ch, keys) in open.iter_mut().enumerate() {
            for (k, slot) in keys.iter_mut().enumerate() {
                if let Some((s, v, t)) = slot.take() {
                    raw.push(RawNote {
                        start: s,
                        end: tick,
                        pitch: k as u8,
                        velocity: v,
                        channel: ch as u8,
                        track: t,
                        clamped: true,
                    });
                }
            }
        }
        if !ended && !track.is_empty() {
            unterminated += 1;
        }
        last_tick = last_tick.max(tick);
    }

    let clamped = raw.iter().filter(|n| n.clamped).count();
    if clamped > 0 {
        warnings.push(format!(
            "{clamped} note(s) had no note-off and were held to the end of their track"
        ));
    }
    if orphan_offs > 0 {
        warnings.push(format!("{orphan_offs} note-off(s) without a matching note-on were ignored"));
    }
    if unterminated > 0 {
        warnings.push(format!("{unterminated} track(s) ended without an End of Track marker; the file may be truncated"));
    }

    // Seconds, once, for every consumer.
    let mut notes: Vec<Note> = raw
        .iter()
        .map(|r| {
            let start = tempo.seconds(r.start);
            let end = tempo.seconds(r.end);
            let mut f = r.channel << flags::CHANNEL_SHIFT;
            if is_black_key(r.pitch) {
                f |= flags::BLACK;
            }
            if r.clamped {
                f |= flags::CLAMPED;
            }
            if r.channel == PERCUSSION_CHANNEL {
                f |= flags::PERCUSSION;
            }
            Note {
                start: start as f32,
                duration: ((end - start) as f32).max(MIN_DURATION),
                sustain: 0.0,
                pitch: r.pitch,
                velocity: r.velocity,
                track: r.track.min(255) as u8,
                flags: f,
            }
        })
        .collect();

    let mut controls: Vec<Control> = controls
        .into_iter()
        .map(|(tick, channel, kind)| Control { time: tempo.seconds(tick), channel, kind })
        .collect();
    controls.sort_by(|a, b| a.time.total_cmp(&b.time));

    apply_pedal(&mut notes, &controls, tempo.seconds(last_tick));

    for n in &notes {
        let t = &mut tracks[n.track as usize];
        t.note_count += 1;
        t.pitch_range = Some(match t.pitch_range {
            None => (n.pitch, n.pitch),
            Some((lo, hi)) => (lo.min(n.pitch), hi.max(n.pitch)),
        });
    }
    for t in &mut tracks {
        if t.instrument.is_none() && t.note_count > 0 {
            t.instrument = Some(match (t.channel, t.program) {
                (Some(PERCUSSION_CHANNEL), _) => "Percussion".into(),
                (_, Some(p)) => gm::program_name(p).into(),
                _ => gm::program_name(0).into(),
            });
        }
    }

    let duration =
        notes.iter().map(|n| (n.end() + n.sustain) as f64).fold(tempo.seconds(last_tick), f64::max);

    Ok(Score {
        notes: NoteTable::new(notes),
        controls,
        tempo,
        tracks,
        duration,
        title,
        copyright,
        warnings,
    })
}

/// Extends notes released while the sustain pedal is down, up to the pedal
/// release or the next strike of the same key, whichever is first.
///
/// Without this, heavily pedaled music looks staccato while sounding legato,
/// and the picture disagrees with the sound.
fn apply_pedal(notes: &mut [Note], controls: &[Control], song_end: f64) {
    // Pedal-down intervals per channel.
    let mut intervals: [Vec<(f64, f64)>; 16] = Default::default();
    let mut down_at: [Option<f64>; 16] = [None; 16];
    for c in controls {
        if let ControlKind::Controller { number: SUSTAIN_PEDAL, value } = c.kind {
            let ch = c.channel as usize & 15;
            match (value >= 64, down_at[ch]) {
                (true, None) => down_at[ch] = Some(c.time),
                (false, Some(d)) => {
                    intervals[ch].push((d, c.time));
                    down_at[ch] = None;
                }
                _ => {}
            }
        }
    }
    for (ch, d) in down_at.iter().enumerate() {
        if let Some(d) = d {
            intervals[ch].push((*d, song_end));
        }
    }
    if intervals.iter().all(Vec::is_empty) {
        return;
    }

    // Sort (channel, pitch, start) so the next strike of a key is adjacent.
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        let (x, y) = (&notes[a], &notes[b]);
        (x.channel(), x.pitch).cmp(&(y.channel(), y.pitch)).then(x.start.total_cmp(&y.start))
    });

    for w in 0..order.len() {
        let i = order[w];
        let n = notes[i];
        if n.has(flags::PERCUSSION) {
            continue;
        }
        let iv = &intervals[n.channel() as usize];
        let release = n.end() as f64;
        // The last interval that began at or before release.
        let k = iv.partition_point(|&(d, _)| d <= release);
        let Some(&(_, up)) = k.checked_sub(1).and_then(|k| iv.get(k)) else { continue };
        if up <= release {
            continue;
        }
        let next_strike = order
            .get(w + 1)
            .map(|&j| &notes[j])
            .filter(|m| m.channel() == n.channel() && m.pitch == n.pitch)
            .map_or(f64::INFINITY, |m| m.start as f64);
        let sustain = (up.min(next_strike) - release) as f32;
        if sustain > 0.0 {
            notes[i].sustain = sustain;
            notes[i].flags |= flags::PEDAL;
        }
    }
}

/// MIDI text has no declared encoding. UTF-8 when valid, else Latin-1, which
/// maps every byte to a character and so never fails.
fn decode_text(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.trim_end_matches('\0').to_owned(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}
