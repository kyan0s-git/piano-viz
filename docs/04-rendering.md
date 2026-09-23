# 04 — Rendering

## Why HDR is not optional

The Embers look is bright notes bleeding light into everything around them.
That effect only works if colors are allowed to exceed 1.0 before the bloom
pass reads them. In an LDR pipeline every bright thing clamps to white, the
bloom picks up a uniform white smear, and the result looks like a blur filter
rather than light.

So the whole chain runs in `Rgba16Float` linear space and tonemaps once at
the very end. A note at 4.0 intensity blooms four times as hard as one at
1.0, even though both display as white. That difference is the entire look.

Everything upstream — colors from the Design, texture samples — is converted
to linear on entry. Mixing sRGB and linear values silently is the classic way
to get muddy gradients and grey-looking glow.

## Layer stack

Rendered bottom to top into one HDR target:

```
  ┌─────────────────────────────────┐
  │ 7. UI overlay      (LDR, after tonemap)
  │ 6. Post chain      bloom → tonemap → vignette → grain
  ├─────────────────────────────────┤  HDR target, Rgba16Float
  │ 5. Particles       additive
  │ 4. Keyboard        keys + press state + glow
  │ 3. Strike line     impact flash
  │ 2. Notes           instanced rounded quads
  │ 1. Background      solid / gradient / image / reflection
  └─────────────────────────────────┘
```

Each layer is one or two draw calls. The whole frame is well under 20 draws
regardless of how dense the MIDI is, because everything that scales with note
count is instanced.

## Coordinates

One convention, defined once, because inconsistency here causes a specific
and maddening class of bug where the notes and the keyboard disagree by a few
pixels at certain aspect ratios.

- Origin bottom-left, y up
- x in `[0, 1]` across the keyboard's full width
- y in pixels from the strike line, positive upward
- The strike line sits at the top edge of the keys

A note's vertical position is:

```
y_bottom = (note.start - t) * pixels_per_second
y_top    = y_bottom + note.duration * pixels_per_second
```

`pixels_per_second = fall_height / lookahead_seconds`. When `note.start == t`
the note's bottom is exactly at the strike line, which is the moment it
sounds and the moment particles spawn. Falling upward-to-downward is just
negative time, so no special-casing for scroll direction.

## Keyboard geometry

Generated once at load, regenerated only when the key range or Design
changes. Not per frame.

The standard 88-key range is A0 (21) to C8 (108), but the range is
configurable and auto-fits to content.

White key x-position is a running count of white keys before it. Black keys
use the real instrument's asymmetric placement: dividing an octave's seven
white keys into twelve equal semitone slots puts semitone `k` at
`(k + 0.5) * 7/12` white-key widths from C. That places C# and F# left of
their gaps and D# and A# right of theirs, as on a real piano. Evenly spacing
them between neighbors is the single most common tell of a hand-rolled
keyboard:

```rust
fn black_center(semitone: u8) -> f32 {
    (semitone as f32 + 0.5) * 7.0 / 12.0
}
```

Black keys are narrower (~0.58 of a white key) and shorter (~0.62), both
configurable. They render after white keys, so they overlap correctly with no
depth buffer needed.

Key state is a 128-entry storage buffer updated per frame — pressed, velocity,
time since press, and the color of the note pressing it. 128 entries is
nothing; there's no need to be clever.

## Note rendering

One instanced draw call for every visible note. The instance data is the
`NoteTable` storage buffer, uploaded once at load and never touched again.

The vertex shader pulls `Note` by instance index, computes the quad corners
from pitch (x, via a key-layout lookup buffer) and time (y, per above), and
the fragment shader does the rounded corners and styling with a signed
distance field:

```wgsl
// Rounded-rect SDF — corner radius without extra geometry or alpha textures.
fn sd_round_box(p: vec2f, b: vec2f, r: f32) -> f32 {
    let q = abs(p) - b + r;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
```

An SDF gives crisp antialiased corners at any size, plus borders and inner
glow from the same distance value, for free. The alternative — geometry per
corner, or a texture atlas — costs more and looks worse when scaled.

Per-note styling from the Design: fill, gradient along length or across
width, border width and color, corner radius, opacity, and an HDR intensity
multiplier that drives how hard it blooms.

### Visible-window query

The CPU's entire per-frame note work:

```rust
let t_end = t + lookahead;                 // furthest note that has entered
let t_start = t - max_extent - tail;       // earliest still-visible note

let lo = notes.partition_point(|n| n.start < t_start);
let hi = notes.partition_point(|n| n.start < t_end);
// draw instances lo..hi
```

Two binary searches. The scan-back by `max_extent` is what handles a very
long note whose start is far off-screen but whose body still covers the
window — without it, held pedal notes vanish. Tracking `max_extent` per
file rather than assuming a bound keeps the range tight for normal music.

Notes are sorted by start, so `lo..hi` is contiguous and draws as one range.
Per-track visibility is handled in the shader by discarding against a track
mask rather than splitting the draw, which keeps it a single call.

## Post-processing chain

### Bloom

Progressive dual-filter downsample/upsample — six mip levels down with a
13-tap filter, then back up with a 9-tap tent, accumulating. This is the
approach from Call of Duty's 2014 presentation, and it's the right one here:
it gives a wide, smooth, stable falloff with no visible ringing and no
temporal flicker on small bright sources, which a naive Gaussian at one scale
cannot do. Cost is roughly 1.3x the base resolution in total sampling.

Parameters: threshold (soft knee), intensity, radius, and tint.

### Tonemap

AgX by default, with ACES and Reinhard as alternatives. AgX because it
desaturates gracefully as values blow out instead of producing the hard
neon-clipped hues ACES gives on saturated bright sources — and saturated
bright sources are exactly what this app draws. Reinhard is there because
it's the most neutral option for anyone compositing later.

### Then, in order

- **Chromatic aberration** — radial, subtle, off by default
- **Vignette** — smoothstep on radial distance
- **Film grain** — animated hash noise, seeded from the playhead time rather
  than the frame number, so it's deterministic and a given moment looks the
  same at any frame rate
- **Dither** — triangular noise before the 8-bit write, to kill banding in
  dark gradients. Cheap, and the alternative is visible stepping in exactly
  the dark backgrounds this app tends to use.

## Background

- **Solid** — one color
- **Gradient** — linear or radial, multi-stop, interpolated in linear space
- **Image** — loaded, mipmapped, with fit/fill/stretch and a tint
- **Reflection** — the classic look: the note area mirrored below the
  keyboard with a vertical fade and optional blur. Implemented by sampling
  the already-rendered scene texture rather than re-rendering geometry.

## Camera

A 2D affine transform applied to the scene, not a real 3D camera:

- **Zoom / pan** — user-controlled, or animated by the Design
- **Impact shake** — decaying oscillation triggered by note velocity, summed
  across simultaneous notes and clamped. Driven by an analytic decay function
  of time-since-hit, not an accumulator, so it stays deterministic and seeks
  correctly like everything else.
- **Drift** — slow low-frequency noise, for motion that keeps a static shot
  from feeling dead

## Antialiasing

Supersampling, not MSAA. Render at an integer 2x-4x and resolve with an
exact box filter. MSAA only antialiases geometry edges; most of the edges here come out
of the SDF fragment shader and from bloom, which MSAA does nothing for.
Supersampling also improves the bloom's sampling quality. It costs more, so
preview defaults to 1x with 2x available, and export defaults to 2x.
