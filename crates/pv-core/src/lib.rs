//! Core types shared by every piano-viz crate.
//!
//! No GPU, audio, or OS dependencies live here, so everything in this crate
//! builds and tests without a graphics device or sound card.

mod note;
mod score;
mod tempo;
mod time;

pub mod gm;

pub use note::{Note, NoteTable, flags};
pub use score::{Control, ControlKind, Score, TrackInfo};
pub use tempo::{TempoMap, TimeSignature};
pub use time::{AudioClock, Clock, FixedClock, FrameClock, Rational};

/// Lowest and highest MIDI pitches on a standard 88-key piano.
pub const PIANO_RANGE: std::ops::RangeInclusive<u8> = 21..=108;

/// True for the five black keys in each octave.
#[inline]
pub const fn is_black_key(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}
