//! Per-frame CPU work. Everything here is bounded by what's on screen, not
//! by the size of the file, and allocates nothing once warmed up.

use crate::layout::KeyLayout;
use crate::uniforms::KeyGpu;
use pv_core::{Note, NoteTable, flags};
use pv_design::Design;

/// Seconds a key's light takes to fade after release.
const RELEASE_FADE: f32 = 0.35;

#[inline]
fn drawable(n: &Note, visible: &[bool], layout: &KeyLayout) -> bool {
    !n.has(flags::PERCUSSION)
        && visible.get(n.track as usize).copied().unwrap_or(true)
        && layout.keys[n.pitch as usize].visible
}

/// Which keys are down at `t`, by which note, and how brightly lit.
pub fn key_states(
    notes: &NoteTable,
    t: f32,
    visible: &[bool],
    layout: &KeyLayout,
    out: &mut [KeyGpu; 128],
) {
    for (k, g) in out.iter_mut().zip(&layout.keys) {
        *k = KeyGpu {
            x0: g.x0,
            x1: g.x1,
            black: if g.black { 1.0 } else { 0.0 },
            visible: if g.visible { 1.0 } else { 0.0 },
            note: -1,
            ..Default::default()
        };
    }
    let all = notes.as_slice();
    // Latest start per key wins, so a re-struck key shows the new note.
    let mut best_start = [f32::NEG_INFINITY; 128];
    for i in notes.overlapping(t - RELEASE_FADE, t + 1e-4) {
        let n = &all[i];
        if n.start > t || !drawable(n, visible, layout) {
            continue;
        }
        let p = n.pitch as usize;
        let press = if t < n.end() { 1.0 } else { (-(t - n.end()) * 12.0).exp() };
        if t >= n.end() + RELEASE_FADE {
            continue;
        }
        let k = &mut out[p];
        // A held note always beats a fading one; among equals, the latest.
        if press > k.press + 1e-6 || (press >= k.press - 1e-6 && n.start > best_start[p]) {
            best_start[p] = n.start;
            k.note = i as i32;
            k.press = press;
            k.age = t - n.start;
            k.velocity = n.velocity as f32 / 127.0;
        }
    }
}

/// Indices of notes that have at least one live particle at `t`.
///
/// Built on the CPU rather than by GPU compaction so the order is fixed,
/// which keeps additive blending — and so every render — bit-reproducible.
pub fn active_notes(
    notes: &NoteTable,
    t: f32,
    max_life: [f32; 3],
    visible: &[bool],
    layout: &KeyLayout,
    out: &mut Vec<u32>,
) {
    out.clear();
    let [hit, hold, release] = max_life;
    let longest = hit.max(hold).max(release);
    if longest <= 0.0 {
        return;
    }
    let all = notes.as_slice();
    for i in notes.starting_in(t - longest - notes.max_extent(), t + 1e-4) {
        let n = &all[i];
        if !drawable(n, visible, layout) {
            continue;
        }
        let end = n.end();
        let alive = (hit > 0.0 && n.start <= t && n.start >= t - hit)
            || (hold > 0.0 && n.start <= t && end + hold >= t)
            || (release > 0.0 && end <= t && end >= t - release);
        if alive {
            out.push(i as u32);
        }
    }
}

/// Camera offset in pixels: decaying shake from recent strikes plus slow
/// drift. Both are analytic in `t`, so seeking lands on the right frame.
pub fn camera_offset(d: &Design, notes: &NoteTable, t: f32, px: f32) -> [f32; 2] {
    let mut off = [0.0f32; 2];
    let s = &d.camera.shake;
    if s.enabled && s.amount > 0.0 {
        let window = 5.0 / s.decay.max(0.1);
        let all = notes.as_slice();
        for i in notes.starting_in(t - window, t + 1e-4) {
            let n = &all[i];
            if n.has(flags::PERCUSSION) {
                continue;
            }
            let age = t - n.start;
            let v = (n.velocity as f32 / 127.0).powf(1.0 + s.velocity_response);
            let a = s.amount * px * v * (-s.decay * age).exp();
            let ph = (i as u32).wrapping_mul(0x9E37_79B1) as f32 / u32::MAX as f32
                * std::f32::consts::TAU;
            off[0] += a * (age * 55.0 + ph).sin();
            off[1] += a * (age * 47.0 + ph * 1.7).cos();
        }
        let len = (off[0] * off[0] + off[1] * off[1]).sqrt();
        let cap = s.amount * px * 3.0;
        if len > cap {
            off = [off[0] * cap / len, off[1] * cap / len];
        }
    }
    let dr = &d.camera.drift;
    if dr.enabled {
        let w = t * dr.speed * std::f32::consts::TAU;
        off[0] += dr.amount * px * ((w + 1.3).sin() + 0.5 * (w * 2.3).sin()) / 1.5;
        off[1] += dr.amount * px * ((w * 0.8).cos() + 0.5 * (w * 1.9 + 0.7).sin()) / 1.5;
    }
    off
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f32, duration: f32, pitch: u8) -> Note {
        Note { start, duration, sustain: 0.0, pitch, velocity: 100, track: 0, flags: 0 }
    }

    fn layout() -> KeyLayout {
        KeyLayout::new(21, 108, 0.58)
    }

    #[test]
    fn held_key_is_pressed_by_its_note() {
        let notes = NoteTable::new(vec![note(1.0, 1.0, 60)]);
        let mut k = [KeyGpu::default(); 128];
        key_states(&notes, 1.5, &[], &layout(), &mut k);
        assert_eq!(k[60].note, 0);
        assert_eq!(k[60].press, 1.0);
        assert!((k[60].age - 0.5).abs() < 1e-6);
        assert_eq!(k[61].note, -1);
    }

    #[test]
    fn released_key_fades_then_goes_dark() {
        let notes = NoteTable::new(vec![note(1.0, 1.0, 60)]);
        let mut k = [KeyGpu::default(); 128];
        key_states(&notes, 2.1, &[], &layout(), &mut k);
        assert!(k[60].press > 0.0 && k[60].press < 1.0);
        key_states(&notes, 3.0, &[], &layout(), &mut k);
        assert_eq!(k[60].note, -1);
    }

    #[test]
    fn restruck_key_shows_the_newer_note() {
        let notes = NoteTable::new(vec![note(0.0, 1.0, 60), note(1.0, 1.0, 60)]);
        let mut k = [KeyGpu::default(); 128];
        key_states(&notes, 1.05, &[], &layout(), &mut k);
        assert_eq!(notes.as_slice()[k[60].note as usize].start, 1.0);
    }

    #[test]
    fn hidden_tracks_do_not_press_keys() {
        let notes = NoteTable::new(vec![note(0.0, 1.0, 60)]);
        let mut k = [KeyGpu::default(); 128];
        key_states(&notes, 0.5, &[false], &layout(), &mut k);
        assert_eq!(k[60].note, -1);
    }

    #[test]
    fn active_notes_cover_burst_hold_and_release() {
        // A: struck just now. B: struck long ago but still held. C: long gone.
        let notes =
            NoteTable::new(vec![note(0.0, 100.0, 40), note(9.5, 0.1, 60), note(1.0, 0.5, 62)]);
        let mut out = Vec::new();
        active_notes(&notes, 10.0, [1.0, 2.0, 0.0], &[], &layout(), &mut out);
        let pitches: Vec<u8> = out.iter().map(|&i| notes.as_slice()[i as usize].pitch).collect();
        assert_eq!(pitches, [40, 60]);
    }

    #[test]
    fn camera_is_still_by_default_and_bounded_when_shaking() {
        let notes =
            NoteTable::new((0..50).map(|i| note(1.0 + i as f32 * 0.001, 0.5, 60)).collect());
        let mut d = Design::default();
        assert_eq!(camera_offset(&d, &notes, 1.1, 1.0), [0.0, 0.0]);
        d.camera.shake.enabled = true;
        let o = camera_offset(&d, &notes, 1.06, 1.0);
        assert!((o[0] * o[0] + o[1] * o[1]).sqrt() <= d.camera.shake.amount * 3.0 + 1e-3);
    }
}

/// The per-frame CPU path must not allocate once warmed up: counted with a
/// thread-local allocator hook so parallel tests don't interfere.
#[cfg(test)]
mod no_alloc {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local!(static COUNT: Cell<usize> = const { Cell::new(0) });

    struct Counting;

    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            COUNT.with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
    }

    #[global_allocator]
    static A: Counting = Counting;

    #[test]
    fn frame_cpu_work_does_not_allocate() {
        let notes: Vec<Note> = (0..20_000)
            .map(|i| Note {
                start: i as f32 * 0.01,
                duration: 0.3,
                sustain: 0.0,
                pitch: 21 + (i % 88) as u8,
                velocity: 90,
                track: 0,
                flags: 0,
            })
            .collect();
        let notes = NoteTable::new(notes);
        let layout = KeyLayout::new(21, 108, 0.58);
        let mut d = Design::default();
        d.camera.shake.enabled = true;
        let mut keys = [KeyGpu::default(); 128];
        let mut active = Vec::with_capacity(4096);
        // Warm up at the densest point, then count.
        active_notes(&notes, 100.0, [1.3, 2.2, 0.0], &[], &layout, &mut active);
        let before = COUNT.with(Cell::get);
        for f in 0..600 {
            let t = 50.0 + f as f32 / 60.0;
            key_states(&notes, t, &[], &layout, &mut keys);
            active_notes(&notes, t, [1.3, 2.2, 0.0], &[], &layout, &mut active);
            std::hint::black_box(camera_offset(&d, &notes, t, 1.0));
        }
        assert_eq!(COUNT.with(Cell::get) - before, 0, "allocated during frames");
    }
}
