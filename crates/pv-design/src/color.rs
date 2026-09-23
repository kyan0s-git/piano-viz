use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// An sRGB color with alpha, written as `"#rrggbb"` or `"#rrggbbaa"`.
///
/// Stored as authored so a Design round-trips byte-for-byte; converted to
/// linear once, at load, via [`Color::linear`]. Mixing sRGB and linear values
/// is how gradients go muddy and glow goes grey.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const WHITE: Self = Self([255, 255, 255, 255]);
    pub const BLACK: Self = Self([0, 0, 0, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.trim().strip_prefix('#')?;
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let nib = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok().map(|v| v * 17);
        match hex.len() {
            3 => Some(Self([nib(0)?, nib(1)?, nib(2)?, 255])),
            4 => Some(Self([nib(0)?, nib(1)?, nib(2)?, nib(3)?])),
            6 => Some(Self([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Self([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }

    /// Linear-light RGB with straight (not premultiplied) alpha.
    pub fn linear(self) -> [f32; 4] {
        let [r, g, b, a] = self.0;
        [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a as f32 / 255.0]
    }

    pub fn with_alpha(self, a: u8) -> Self {
        Self([self.0[0], self.0[1], self.0[2], a])
    }
}

pub fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.0;
        if a == 255 {
            write!(f, "#{r:02x}{g:02x}{b:02x}")
        } else {
            write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "invalid color {s:?}; expected \"#rrggbb\" or \"#rrggbbaa\""
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_hex_forms() {
        assert_eq!(Color::parse("#ff6b35"), Some(Color([255, 0x6b, 0x35, 255])));
        assert_eq!(Color::parse("#ff6b35c0"), Some(Color([255, 0x6b, 0x35, 0xc0])));
        assert_eq!(Color::parse("#fff"), Some(Color::WHITE));
        assert_eq!(Color::parse("#0008"), Some(Color([0, 0, 0, 0x88])));
        assert_eq!(Color::parse("ff6b35"), None);
        assert_eq!(Color::parse("#ff6b3"), None);
        assert_eq!(Color::parse("#gg0000"), None);
    }

    #[test]
    fn display_round_trips() {
        for s in ["#ff6b35", "#ff6b35c0", "#000000"] {
            assert_eq!(Color::parse(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn srgb_midgrey_is_about_a_fifth_linear() {
        // sRGB 50% grey is ~21% linear light: the whole reason for converting.
        assert!((srgb_to_linear(128) - 0.2158).abs() < 1e-3);
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
    }
}
