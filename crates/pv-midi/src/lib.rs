//! MIDI file parsing into a [`Score`].
//!
//! Every malformed-but-real-world case has a defined behavior rather than an
//! error; see `docs/03-midi.md` for the table this implements.

pub mod demo;
mod parse;
pub mod write;

pub use parse::parse;

use pv_core::Score;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum MidiError {
    #[error("could not read {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("not a readable MIDI file: {0}")]
    Invalid(String),
}

/// Read and parse a MIDI file from disk.
pub fn load(path: impl AsRef<Path>) -> Result<Score, MidiError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .map_err(|source| MidiError::Io { path: path.display().to_string(), source })?;
    parse(&bytes)
}
