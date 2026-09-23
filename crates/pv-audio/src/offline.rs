//! Faster-than-real-time rendering for export: the same sequencer and synth
//! as playback, so the exported audio is what was auditioned.

use crate::sequencer::{Player, Sequence, SequenceOptions, TrackMask};
use crate::{AudioError, limit, new_synth};
use pv_core::Score;
use rustysynth::SoundFont;
use std::sync::Arc;

pub struct OfflineOptions {
    pub sample_rate: u32,
    pub sequence: SequenceOptions,
    pub muted: TrackMask,
    pub volume: f32,
}

impl Default for OfflineOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            sequence: SequenceOptions::default(),
            muted: TrackMask::default(),
            volume: 1.0,
        }
    }
}

/// Render `start..end` seconds of `score` to interleaved stereo f32.
/// `progress` receives 0..=1 as rendering proceeds. Deterministic: the same
/// inputs always produce the same samples.
pub fn render(
    score: &Score,
    font: &Arc<SoundFont>,
    opts: &OfflineOptions,
    start: f64,
    end: f64,
    mut progress: impl FnMut(f32),
) -> Result<Vec<f32>, AudioError> {
    let sr = opts.sample_rate;
    let mut synth = new_synth(font, sr)?;
    let seq = Sequence::new(score, sr, opts.sequence);
    let mut player = Player::default();
    player.muted = opts.muted;
    let from = (start.max(0.0) * sr as f64).round() as u64;
    player.seek(&seq, &mut synth, from, &seq.chase(from));

    let total = ((end - start).max(0.0) * sr as f64).round() as usize;
    let mut out = Vec::with_capacity(total * 2);
    const BLOCK: usize = 4096;
    let (mut l, mut r) = (vec![0f32; BLOCK], vec![0f32; BLOCK]);
    let mut done = 0;
    while done < total {
        let n = BLOCK.min(total - done);
        player.render(&seq, &mut synth, &mut l[..n], &mut r[..n]);
        for i in 0..n {
            out.push(limit(l[i] * opts.volume));
            out.push(limit(r[i] * opts.volume));
        }
        done += n;
        progress(done as f32 / total.max(1) as f32);
    }
    Ok(out)
}
