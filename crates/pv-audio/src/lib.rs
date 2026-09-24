//! Sound for piano-viz: SoundFont synthesis, real-time playback that owns
//! the master clock, and offline rendering for export. See docs/06-audio.md.

mod engine;
mod font;
pub mod offline;
pub mod piano;
pub mod sequencer;
pub mod sf2;
pub mod wav;

pub use engine::{Engine, EngineClock, LiveSender};
pub use font::{builtin_font, has_drums, load_font};
pub use rustysynth::SoundFont;
pub use sequencer::{SequenceOptions, TrackMask};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio device: {0}")]
    Device(String),
    #[error("SoundFont: {0}")]
    SoundFont(String),
}

/// rustysynth defaults to 0.5, which leaves typical piano music peaking
/// around -16 dBFS. This puts the demo near -6 dBFS; the limiter below
/// catches the rare dense chord that would go over.
const MASTER_GAIN: f32 = 3.0;

pub(crate) fn new_synth(
    font: &std::sync::Arc<SoundFont>,
    sample_rate: u32,
) -> Result<rustysynth::Synthesizer, AudioError> {
    let mut s = rustysynth::SynthesizerSettings::new(sample_rate as i32);
    s.maximum_polyphony = 128;
    let mut synth = rustysynth::Synthesizer::new(font, &s)
        .map_err(|e| AudioError::SoundFont(format!("{e:?}")))?;
    synth.set_master_volume(MASTER_GAIN);
    Ok(synth)
}

/// Soft limiter: transparent below 0.8, then a tanh knee that approaches
/// but never reaches full scale. Stateless, so offline renders stay
/// bit-reproducible and identical to playback.
#[inline]
pub fn limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}
