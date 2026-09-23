# 05 — Particles

The particle bursts are what make an Embers-style video read as "produced"
rather than "a MIDI player with colors." They also determine whether the
timeline can be scrubbed, so this system's design has consequences well
beyond how it looks.

## The problem with the obvious approach

A conventional GPU particle system keeps a buffer of live particles and steps
it each frame: spawn, integrate, retire. It works, and it's what most engines
do. But it has three properties that are bad here specifically:

1. **Seeking is broken.** Jump the playhead to 2:30 and the buffer holds
   particles from wherever you were. To make it correct you'd have to
   simulate forward from the start — seconds of stalling on every scrub, in a
   tool whose whole point is immediate visual feedback.
2. **Renders aren't reproducible.** The state depends on the exact sequence
   of frames and timesteps that led to it. Export at a different framerate
   and you get different particles.
3. **Preview and export diverge.** Preview steps with variable `dt` from real
   frame timing; export steps with fixed `dt`. Same moment, different picture
   — which breaks the promise that what you preview is what you render.

All three come from the same root cause: state. So we remove the state.

## The insight

A particle is born when a note is struck. We know every note's strike time
from the MIDI, before playback even starts. So for any time `t`, the set of
live particles is *derivable* — it's every particle from every note struck in
the window `[t - max_lifetime, t]`.

That's exactly the visible-note query we're already doing for note rendering.

If a particle's position is a closed-form function of its seed and its age,
there is nothing to store and nothing to step:

```
particle(note, i, t):
    age = t - note.start
    if age < 0 or age > lifetime: not alive
    seed = hash(note_index, i)
    position = f(seed, age)
```

Seeking becomes free. Renders become reproducible by construction. Preview
and export produce identical output because both are evaluating the same
function at the same `t`. The three problems don't get solved so much as they
stop existing.

## Motion

Under constant acceleration with linear drag, the position integral has a
closed form:

```wgsl
// Closed-form position under gravity + linear drag, at time `age`.
// Equivalent to integrating dv/dt = g - k*v, but with no timestep.
fn particle_pos(p0: vec3f, v0: vec3f, g: vec3f, k: f32, age: f32) -> vec3f {
    if (k < 1e-4) {                      // drag-free: plain ballistic
        return p0 + v0 * age + 0.5 * g * age * age;
    }
    let e = 1.0 - exp(-k * age);
    return p0 + (v0 + g / k) * (e / k) - g * age / k;
}
```

Turbulence is added as a sum of two sinusoids with seed-derived phase and
frequency. It's not physically a curl-noise field, but it's bounded, cheap,
closed-form, and visually reads as drift — which is all it needs to do:

```wgsl
fn turbulence(seed: u32, age: f32, amp: f32) -> vec2f {
    let ph = vec2f(hash_f(seed ^ 0x9E37u), hash_f(seed ^ 0x85EBu)) * TAU;
    let fr = 1.0 + hash_f(seed ^ 0xC2B2u) * 2.0;
    return vec2f(sin(ph.x + age * fr), cos(ph.y + age * fr * 0.7)) * amp * age;
}
```

Scaling by `age` keeps particles coherent at the moment of the burst and lets
them wander apart as they travel, which is how real sparks behave.

## Emitters

Emitters attach to events, not to objects:

| Event | Fires when | Typical use |
|---|---|---|
| `NoteHit` | note crosses the strike line (in live mode: the key goes down) | the burst — the signature effect |
| `NoteHold` | continuously while sounding | rising embers from a held key |
| `NoteRelease` | note ends | a soft puff on release |

`NoteHold` needs care: it's continuous, so its "spawn time" is a sub-stream
within the note's duration. Particle `i` of a hold emitter spawns at
`note.start + i / rate`, which is still closed-form, so it stays stateless.
The live set for a held note is the sub-range of `i` whose spawn times fall
in the last `lifetime` seconds — computed with two divisions, no search.

Per-emitter parameters: count or rate, lifetime with variance, initial speed
and spread, gravity, drag, size curve, opacity curve, color source, blend
mode, and velocity response (how much the MIDI note's velocity scales count,
speed, and size — this is what makes a loud passage visibly explode).

## Drawing

One instanced draw for all particles of all emitters.

Instance index decomposes to `(note_slot, particle_index)` by division against
a per-emitter stride. `note_slot` indexes a compacted list of recently-struck
notes that a small compute pass builds each frame from the visible-window
range — a prefix-sum compaction, so the draw isn't mostly dead instances.

Particles that aren't alive collapse their quad to zero area in the vertex
shader, which the rasterizer discards for free — cheaper than any branch or
indirect-draw arrangement at this scale.

Rendering is additive into the HDR target, with intensity above 1.0 so
particles drive the bloom. Round soft-edged sprites are generated
analytically in the fragment shader; no texture, so no atlas to load, no
filtering artifacts, and resolution independence.

## Cost

The live particle count is bounded by:

```
notes_in_window × particles_per_note
```

For a dense piece — say 40 notes/second, 1-second lifetime, 64 particles per
note — that's about 2,560 live particles. Even a pathological black MIDI at
1000 notes/second with the same settings gives 64,000, which is still a
single draw call of trivially simple geometry.

There is no buffer to allocate, no compaction of dead particles, no
double-buffering, and no upload. Memory cost is the compacted note-slot list:
a few kilobytes.

## What this gives up

Being honest about the trade, because it is a real one.

Stateless particles cannot do anything that depends on their own history:
collisions, inter-particle forces, trails that respond to what they've
already passed through, or true curl-noise advection. Those need state.

For this application that's an easy trade — none of those effects appear in
the reference material, and instant scrubbing plus reproducible renders are
worth far more than particle collision in a music visualizer.

If a future effect genuinely needs it, the escape hatch is a separate
stateful emitter type that warms up on seek by simulating the last
`lifetime` seconds. That's bounded work, not simulate-from-zero, so it stays
tolerable. It is deliberately not in v1.
