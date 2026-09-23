/// Converts MIDI ticks to seconds across tempo changes.
///
/// Built once at load. Each conversion is a binary search plus a multiply,
/// rather than a walk over every tempo change before the tick.
#[derive(Clone, Debug)]
pub struct TempoMap {
    kind: Kind,
    time_signatures: Vec<TimeSignature>,
}

#[derive(Clone, Debug)]
enum Kind {
    /// Ticks per quarter note; tempo events apply.
    Metrical { tpqn: u16, segments: Vec<Segment> },
    /// Absolute time base; tempo events are ignored.
    Timecode { ticks_per_second: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    tick: u64,
    /// Elapsed seconds at `tick`. f64: this accumulates, so error compounds.
    seconds: f64,
    us_per_quarter: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeSignature {
    pub seconds: f64,
    pub numerator: u8,
    /// Actual denominator (4, 8, ...), not the MIDI power-of-two exponent.
    pub denominator: u8,
}

/// MIDI's default tempo when a file never sets one: 120 BPM.
pub const DEFAULT_US_PER_QUARTER: u32 = 500_000;

impl TempoMap {
    /// Build from `(tick, microseconds per quarter)` changes, in any order.
    pub fn metrical(tpqn: u16, mut changes: Vec<(u64, u32)>) -> Self {
        let tpqn = tpqn.max(1);
        changes.sort_by_key(|&(tick, _)| tick);
        let mut segments =
            vec![Segment { tick: 0, seconds: 0.0, us_per_quarter: DEFAULT_US_PER_QUARTER }];
        for (tick, us) in changes {
            let last = *segments.last().unwrap();
            let seconds = last.seconds + ticks_to_secs(tick - last.tick, last.us_per_quarter, tpqn);
            let seg = Segment { tick, seconds, us_per_quarter: us.max(1) };
            // Several changes on one tick: the last one wins.
            if last.tick == tick {
                *segments.last_mut().unwrap() = seg;
            } else {
                segments.push(seg);
            }
        }
        Self { kind: Kind::Metrical { tpqn, segments }, time_signatures: Vec::new() }
    }

    /// SMPTE timing: `fps` frames per second, `ticks_per_frame` subdivisions.
    pub fn timecode(fps: f64, ticks_per_frame: u8) -> Self {
        let ticks_per_second = (fps * ticks_per_frame.max(1) as f64).max(1.0);
        Self { kind: Kind::Timecode { ticks_per_second }, time_signatures: Vec::new() }
    }

    pub fn seconds(&self, tick: u64) -> f64 {
        match &self.kind {
            Kind::Timecode { ticks_per_second } => tick as f64 / ticks_per_second,
            Kind::Metrical { tpqn, segments } => {
                let i = segments.partition_point(|s| s.tick <= tick) - 1;
                let s = segments[i];
                s.seconds + ticks_to_secs(tick - s.tick, s.us_per_quarter, *tpqn)
            }
        }
    }

    /// Beats per minute in effect at `seconds`. Timecode files report 120.
    pub fn bpm_at(&self, seconds: f64) -> f64 {
        match &self.kind {
            Kind::Timecode { .. } => 120.0,
            Kind::Metrical { segments, .. } => {
                let i = segments.partition_point(|s| s.seconds <= seconds).max(1) - 1;
                60_000_000.0 / segments[i].us_per_quarter as f64
            }
        }
    }

    pub fn set_time_signatures(&mut self, mut sigs: Vec<TimeSignature>) {
        sigs.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
        self.time_signatures = sigs;
    }

    pub fn time_signatures(&self) -> &[TimeSignature] {
        &self.time_signatures
    }

    /// Bar start times up to `end`, for timeline markers.
    pub fn bars(&self, end: f64) -> Vec<f64> {
        let mut out = Vec::new();
        let mut t = 0.0;
        while t < end && out.len() < 100_000 {
            out.push(t);
            let sig = self
                .time_signatures
                .iter()
                .rev()
                .find(|s| s.seconds <= t + 1e-9)
                .copied()
                .unwrap_or(TimeSignature { seconds: 0.0, numerator: 4, denominator: 4 });
            let quarters = sig.numerator as f64 * 4.0 / sig.denominator.max(1) as f64;
            t += (quarters * 60.0 / self.bpm_at(t)).max(0.05);
        }
        out
    }
}

impl Default for TempoMap {
    fn default() -> Self {
        Self::metrical(480, Vec::new())
    }
}

#[inline]
fn ticks_to_secs(ticks: u64, us_per_quarter: u32, tpqn: u16) -> f64 {
    ticks as f64 * us_per_quarter as f64 / (tpqn as f64 * 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn default_tempo_is_120_bpm() {
        let m = TempoMap::metrical(480, vec![]);
        assert!(close(m.seconds(480), 0.5));
        assert!(close(m.seconds(960 * 4), 4.0));
    }

    #[test]
    fn tempo_change_mid_song() {
        // 120 BPM for one quarter, then 60 BPM.
        let m = TempoMap::metrical(480, vec![(480, 1_000_000)]);
        assert!(close(m.seconds(480), 0.5));
        assert!(close(m.seconds(960), 1.5));
        assert!(close(m.seconds(720), 1.0));
    }

    #[test]
    fn many_changes_accumulate() {
        let changes = (1..=100).map(|i| (i * 100, 400_000 + i as u32 * 1000)).collect();
        let m = TempoMap::metrical(100, changes);
        // Walk it the slow way and compare.
        let mut expected = 0.0;
        let mut us = DEFAULT_US_PER_QUARTER;
        for i in 1..=100u64 {
            expected += us as f64 / 1e6;
            us = 400_000 + i as u32 * 1000;
        }
        expected += 3.0 * us as f64 / 1e6;
        assert!(close(m.seconds(10_300), expected));
    }

    #[test]
    fn same_tick_changes_last_wins() {
        let m = TempoMap::metrical(480, vec![(0, 250_000), (0, 1_000_000)]);
        assert!(close(m.seconds(480), 1.0));
    }

    #[test]
    fn timecode_ignores_tempo() {
        let m = TempoMap::timecode(25.0, 40);
        assert!(close(m.seconds(1000), 1.0));
    }

    #[test]
    fn zero_tpqn_does_not_divide_by_zero() {
        let m = TempoMap::metrical(0, vec![]);
        assert!(m.seconds(10).is_finite());
    }

    #[test]
    fn bars_follow_time_signature() {
        let mut m = TempoMap::metrical(480, vec![]);
        m.set_time_signatures(vec![TimeSignature { seconds: 0.0, numerator: 3, denominator: 4 }]);
        let bars = m.bars(4.0);
        assert!(close(bars[1], 1.5));
    }
}
