use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A source of playhead time.
///
/// Preview and export differ only in which implementation they hand the
/// renderer. Nothing downstream can tell them apart — that is what makes the
/// preview match the render.
pub trait Clock {
    /// Current playhead position in seconds.
    fn now(&self) -> f64;
}

/// Preview time: whatever the audio thread says has been played.
///
/// The audio callback is the only authoritative measure of elapsed music, so
/// video follows it and never the other way round. That is why playback
/// cannot drift.
#[derive(Clone, Debug)]
pub struct AudioClock {
    samples: Arc<AtomicU64>,
    sample_rate: u32,
    /// Output latency: sound generated now is heard this many seconds later.
    latency: f64,
    /// Where sample 0 sits on the song timeline.
    origin: f64,
}

impl AudioClock {
    pub fn new(samples: Arc<AtomicU64>, sample_rate: u32) -> Self {
        Self { samples, sample_rate: sample_rate.max(1), latency: 0.0, origin: 0.0 }
    }

    pub fn with_latency(mut self, seconds: f64) -> Self {
        self.latency = seconds.max(0.0);
        self
    }

    pub fn with_origin(mut self, seconds: f64) -> Self {
        self.origin = seconds;
        self
    }
}

impl Clock for AudioClock {
    fn now(&self) -> f64 {
        // Acquire pairs with the audio thread's Release store, so anything it
        // wrote before publishing (key states) is visible alongside it.
        let played = self.samples.load(Ordering::Acquire) as f64 / self.sample_rate as f64;
        self.origin + played - self.latency
    }
}

/// An exact frame rate, so 29.97 is 30000/1001 rather than an approximation
/// that drifts over a long render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den.max(1) as f64
    }

    /// Parses "60", "59.94", "29.97", "30000/1001".
    pub fn parse_fps(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some((n, d)) = s.split_once('/') {
            let r = Self::new(n.trim().parse().ok()?, d.trim().parse().ok()?);
            return (r.num > 0 && r.den > 0).then_some(r);
        }
        match s {
            "23.976" | "23.98" => return Some(Self::new(24000, 1001)),
            "29.97" => return Some(Self::new(30000, 1001)),
            "59.94" => return Some(Self::new(60000, 1001)),
            _ => {}
        }
        let v: f64 = s.parse().ok()?;
        (v > 0.0 && v <= 1000.0 && v.fract() == 0.0).then(|| Self::new(v as u32, 1))
    }
}

/// Export time: frame `n` is exactly `n * den / num` seconds.
#[derive(Clone, Copy, Debug)]
pub struct FrameClock {
    pub frame: u64,
    pub fps: Rational,
    /// Song time of frame 0.
    pub start: f64,
}

impl FrameClock {
    pub fn new(fps: Rational, start: f64) -> Self {
        Self { frame: 0, fps, start }
    }

    pub fn at(fps: Rational, start: f64, frame: u64) -> Self {
        Self { frame, fps, start }
    }
}

impl Clock for FrameClock {
    fn now(&self) -> f64 {
        // Computed fresh from the integer frame, never accumulated.
        self.start + (self.frame * self.fps.den as u64) as f64 / self.fps.num.max(1) as f64
    }
}

/// A clock stopped at one moment: scrubbing, tests, thumbnails.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub f64);

impl Clock for FixedClock {
    fn now(&self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_clock_is_exact_at_ntsc_rates() {
        let fps = Rational::new(30000, 1001);
        // One hour of 29.97: 107892 frames is 3599.9964 s. Accumulating a
        // float step would be off by far more than a frame by now.
        let c = FrameClock::at(fps, 0.0, 107_892);
        assert!((c.now() - 107_892.0 * 1001.0 / 30000.0).abs() < 1e-9);
    }

    #[test]
    fn audio_clock_applies_latency_and_origin() {
        let s = Arc::new(AtomicU64::new(48_000));
        let c = AudioClock::new(s, 48_000).with_latency(0.01).with_origin(10.0);
        assert!((c.now() - 10.99).abs() < 1e-12);
    }

    #[test]
    fn parses_fps() {
        assert_eq!(Rational::parse_fps("60"), Some(Rational::new(60, 1)));
        assert_eq!(Rational::parse_fps("59.94"), Some(Rational::new(60000, 1001)));
        assert_eq!(Rational::parse_fps("24000/1001"), Some(Rational::new(24000, 1001)));
        assert_eq!(Rational::parse_fps("0"), None);
        assert_eq!(Rational::parse_fps("12.5"), None);
    }
}
