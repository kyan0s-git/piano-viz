# piano-viz design

The complete plan, written before any code. Read in order the first time;
after that each document stands alone.

| # | Document | What it settles |
|---|---|---|
| 01 | [Vision & scope](01-vision.md) | What we're building, what we're not, how it compares |
| 02 | [Architecture](02-architecture.md) | Crates, threads, clocks, data flow |
| 03 | [MIDI pipeline](03-midi.md) | Parsing, tempo maps, the note model, edge cases |
| 04 | [Rendering](04-rendering.md) | GPU pipeline, layers, notes, keyboard, post-processing |
| 05 | [Particles](05-particles.md) | The stateless particle system — the signature effect |
| 06 | [Audio](06-audio.md) | Synthesis, the master clock, offline rendering |
| 07 | [Live mode](07-live-mode.md) | MIDI input devices, latency budget |
| 08 | [Export](08-export.md) | Deterministic rendering, encoders, alpha output |
| 09 | [Design format](09-design-format.md) | The theme schema users edit and share |
| 10 | [UI & UX](10-ui-ux.md) | Layout, panels, shortcuts, interaction |
| 11 | [Performance](11-performance.md) | Budgets and how we hold them |
| 12 | [Testing](12-testing.md) | Golden images, determinism, fuzzing |
| 13 | [Roadmap](13-roadmap.md) | Milestones and ordering |

## The three decisions everything else follows from

**1. Audio is the master clock.** The audio callback owns the playhead.
Video reads it. This is why playback never drifts — see
[Architecture](02-architecture.md#clocks).

**2. Preview and export share one render path.** The renderer is a pure
function of `(scene, time)`. Preview supplies time from the audio clock,
export from a frame counter. Nothing else differs, so what you preview is
exactly what you get — see [Export](08-export.md).

**3. Particles are stateless.** A particle's position is a closed-form
function of its seed and age, and a note's hit time is known from the MIDI.
So the live particle set is *derived* from the visible note set, with no
simulation buffer at all. Seeking is instant and renders are reproducible —
see [Particles](05-particles.md).

Each of these is load-bearing. Breaking any one of them breaks something
user-visible: drift, preview/export mismatch, or unscrubbable timelines.
