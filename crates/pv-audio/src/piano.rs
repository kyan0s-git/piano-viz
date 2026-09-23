//! The built-in piano, synthesized at startup rather than shipped as a
//! sample library: zero bytes of audio in the binary, and no licensing
//! question. It's a respectable default; any SF2 can replace it.
//!
//! The model is additive and physically motivated:
//! - stiff-string inharmonicity, f_n = n f0 sqrt(1 + B n^2), with B rising
//!   toward the treble
//! - a hammer striking ~1/8 of the way along the string, which notches the
//!   partials near every eighth
//! - two-stage decay (fast "prompt" sound, slow "aftersound") and higher
//!   partials dying faster
//! - up to three slightly detuned unison strings, whose beating is much of
//!   what makes a piano sound like a piano rather than an organ
//! - a short filtered-noise hammer transient
//! - two velocity layers, since brightness, not just loudness, changes with
//!   how hard a key is struck

use crate::sf2::{self, Sample, Zone, generator as g};

pub const SAMPLE_RATE: u32 = 32_000;
/// A sample every six semitones keeps pitch-shifting to +/- 3 semitones,
/// where resampling artifacts stay inaudible.
const ROOT_STEP: u8 = 6;
const LOWEST: u8 = 21;
const HIGHEST: u8 = 108;
/// Velocity at which the bright layer takes over.
const LAYER_SPLIT: u8 = 80;

/// Build the piano as SF2 bytes. Takes a few hundred milliseconds; callers
/// run it off the UI thread.
pub fn soundfont() -> Vec<u8> {
    let roots: Vec<u8> = (LOWEST..=HIGHEST).step_by(ROOT_STEP as usize).collect();
    let jobs: Vec<(u8, bool)> = roots.iter().flat_map(|&r| [(r, false), (r, true)]).collect();

    // Synthesis is embarrassingly parallel: one job per sample.
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
    let mut data: Vec<Vec<i16>> = vec![Vec::new(); jobs.len()];
    std::thread::scope(|s| {
        for (chunk_jobs, chunk_out) in jobs
            .chunks(jobs.len().div_ceil(threads))
            .zip(data.chunks_mut(jobs.len().div_ceil(threads)))
        {
            s.spawn(move || {
                for (&(root, hard), out) in chunk_jobs.iter().zip(chunk_out) {
                    *out = synthesize(root, hard);
                }
            });
        }
    });

    let samples: Vec<Sample> = jobs
        .iter()
        .zip(data)
        .map(|(&(root, hard), data)| Sample {
            name: format!("P{root}{}", if hard { "f" } else { "p" }),
            data,
            rate: SAMPLE_RATE,
            root,
        })
        .collect();

    let mut zones = Vec::new();
    for (i, &root) in roots.iter().enumerate() {
        let lo = if i == 0 { 0 } else { root - ROOT_STEP / 2 };
        let hi = if i + 1 == roots.len() { 127 } else { root + ROOT_STEP / 2 - 1 };
        let t = (root - LOWEST) as f32 / (HIGHEST - LOWEST) as f32;
        // Bass left, treble right, from the player's seat — gently.
        let pan = ((t - 0.5) * 500.0) as i16;
        // Dampers stop the treble faster than the heavy bass strings.
        let release = sf2::timecents(0.55 - 0.35 * t);
        for (layer, vel) in [(0usize, (0, LAYER_SPLIT - 1)), (1, (LAYER_SPLIT, 127))] {
            zones.push(Zone {
                keys: (lo, hi),
                velocities: vel,
                sample: i * 2 + layer,
                generators: vec![
                    (g::PAN, pan),
                    (g::ATTACK_VOL_ENV, sf2::timecents(0.001)),
                    // No envelope decay: the sample carries the piano's own.
                    (g::DECAY_VOL_ENV, sf2::timecents(0.001)),
                    (g::SUSTAIN_VOL_ENV, 0),
                    (g::RELEASE_VOL_ENV, release),
                    (g::SAMPLE_MODES, 0),
                ],
            });
        }
    }
    sf2::build("piano-viz Piano", &samples, &zones, &[(g::REVERB_SEND, 180), (g::CHORUS_SEND, 0)])
}

/// One sample: `root` at soft (`hard = false`) or loud velocity.
pub fn synthesize(root: u8, hard: bool) -> Vec<i16> {
    let sr = SAMPLE_RATE as f32;
    let k = root as f32;
    let t = (k - LOWEST as f32) / (HIGHEST - LOWEST) as f32;
    let f0 = 440.0 * 2f32.powf((k - 69.0) / 12.0);
    let seconds = 9.0 - 6.8 * t.powf(0.8);
    let len = (seconds * sr) as usize;

    let b = 0.00007 * (0.058 * (k - LOWEST as f32)).exp();
    let tilt = if hard { 0.85 } else { 1.35 };
    let cutoff = if hard { 5200.0 + 3.0 * f0 } else { 1900.0 + 2.0 * f0 };
    let t1 = 3.6 * (-0.0265 * (k - LOWEST as f32)).exp(); // fundamental's 1/e time
    let strings: &[f32] = match root {
        0..=32 => &[0.0],
        33..=44 => &[-0.6, 0.6],
        _ => &[-0.8, 0.1, 0.9],
    };
    let nyquist_guard = (sr * 0.45).min(14_000.0);
    let hammer_pos = 1.0 / 8.3;

    let mut acc = vec![0f32; len];
    for n in 1..=64u32 {
        let nf = n as f32;
        let fn_ = nf * f0 * (1.0 + b * nf * nf).sqrt();
        if fn_ > nyquist_guard {
            break;
        }
        let comb = (std::f32::consts::PI * nf * hammer_pos).sin().abs().max(0.06);
        let amp = comb / nf.powf(tilt) * (-fn_ / cutoff).exp();
        if amp < 1e-4 {
            continue;
        }
        let decay = (1.0 + 0.9 * (fn_ / 1000.0).powf(1.2)) / t1;
        for (si, &cents) in strings.iter().enumerate() {
            let f = fn_ * 2f32.powf(cents * (0.6 + 0.8 * t) / 1200.0);
            // Each string's own slightly different decay spreads the beating.
            let d = decay * (1.0 + 0.07 * si as f32);
            add_partial(
                &mut acc,
                f / sr,
                amp / strings.len() as f32,
                d / sr,
                (n * 7 + si as u32) as f32 * 0.37,
            );
        }
    }

    // Hammer: a short burst of low-passed noise.
    let mut rng = 0x2545_F491u32 ^ (root as u32 * 7919) ^ hard as u32;
    let (mut lp, a) =
        (0.0f32, (-2.0 * std::f32::consts::PI * (if hard { 3200.0 } else { 1500.0 }) / sr).exp());
    let thump = if hard { 0.09 } else { 0.035 };
    for (i, s) in acc.iter_mut().enumerate().take((0.04 * sr) as usize) {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let white = rng as f32 / u32::MAX as f32 * 2.0 - 1.0;
        lp = a * lp + (1.0 - a) * white;
        *s += lp * thump * (-(i as f32) / (0.007 * sr)).exp();
    }

    // Attack ramp and a tail fade so nothing clicks.
    let ramp = (0.0015 * sr) as usize;
    let fade = (0.08 * sr) as usize;
    for (i, v) in acc.iter_mut().take(ramp).enumerate() {
        *v *= 0.5 - 0.5 * (std::f32::consts::PI * i as f32 / ramp as f32).cos();
    }
    for (i, v) in acc.iter_mut().rev().take(fade).enumerate() {
        *v *= i as f32 / fade as f32;
    }

    let peak = acc.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    let gain = 0.85 * i16::MAX as f32 / peak;
    acc.iter().map(|v| (v * gain).round() as i16).collect()
}

/// Add one decaying partial with a two-stage envelope, using a rotating
/// phasor instead of `sin()` per sample: a few multiplies per sample, which
/// is what makes synthesizing a whole piano at startup affordable.
fn add_partial(
    out: &mut [f32],
    cycles_per_sample: f32,
    amp: f32,
    decay_per_sample: f32,
    phase: f32,
) {
    let w = std::f64::consts::TAU * cycles_per_sample as f64;
    let (rs, rc) = (w.sin() as f32, w.cos() as f32);
    let (mut s, mut c) = (phase.sin(), phase.cos());
    // Prompt sound decays 2.5x faster than the aftersound.
    let (fast, slow) = ((-decay_per_sample * 2.5).exp(), (-decay_per_sample * 0.6).exp());
    let (mut ef, mut es) = (0.72 * amp, 0.28 * amp);
    for (i, o) in out.iter_mut().enumerate() {
        *o += s * (ef + es);
        let ns = s * rc + c * rs;
        c = c * rc - s * rs;
        s = ns;
        ef *= fast;
        es *= slow;
        if i & 1023 == 1023 {
            // Renormalize so rounding can't grow or shrink the phasor.
            let m = (s * s + c * c).sqrt();
            s /= m;
            c /= m;
            if ef + es < 1e-5 * amp.max(1e-3) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_decay_and_do_not_clip() {
        let s = synthesize(60, true);
        let peak = s.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak > 20_000 && peak < i16::MAX as u16);
        let rms = |r: &[i16]| {
            (r.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / r.len() as f64).sqrt()
        };
        let n = SAMPLE_RATE as usize;
        assert!(rms(&s[n / 10..n / 5]) > rms(&s[2 * n..2 * n + n / 10]) * 3.0, "should decay");
    }

    #[test]
    fn loud_layer_is_brighter() {
        // More energy in the high end: compare sample-to-sample differences,
        // a crude high-pass, normalized by overall level.
        let hf = |s: &[i16]| {
            let d: f64 = s.windows(2).take(8000).map(|w| (w[1] as f64 - w[0] as f64).abs()).sum();
            d / s.iter().take(8000).map(|&v| (v as f64).abs()).sum::<f64>()
        };
        assert!(hf(&synthesize(60, true)) > hf(&synthesize(60, false)) * 1.2);
    }
}
