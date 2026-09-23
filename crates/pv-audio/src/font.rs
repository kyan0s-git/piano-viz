use crate::AudioError;
use rustysynth::SoundFont;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// The generated piano. Built on first use and shared for the life of the
/// process.
pub fn builtin_font() -> Arc<SoundFont> {
    static FONT: OnceLock<Arc<SoundFont>> = OnceLock::new();
    FONT.get_or_init(|| {
        let bytes = crate::piano::soundfont();
        Arc::new(
            SoundFont::new(&mut std::io::Cursor::new(bytes)).expect("generated SoundFont is valid"),
        )
    })
    .clone()
}

/// Load a user SoundFont (SF2).
pub fn load_font(path: impl AsRef<Path>) -> Result<Arc<SoundFont>, AudioError> {
    let path = path.as_ref();
    let file = std::fs::File::open(path)
        .map_err(|e| AudioError::SoundFont(format!("{}: {e}", path.display())))?;
    let mut reader = std::io::BufReader::new(file);
    SoundFont::new(&mut reader)
        .map(Arc::new)
        .map_err(|e| AudioError::SoundFont(format!("{}: {e:?}", path.display())))
}

/// Whether a font has a drum kit (bank 128), so percussion is worth playing.
pub fn has_drums(font: &SoundFont) -> bool {
    font.get_presets().iter().any(|p| p.get_bank_number() == 128)
}
