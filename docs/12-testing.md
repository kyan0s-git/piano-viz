# 12 — Testing

A visualizer is mostly untestable by conventional means — you can't assert
that something looks good. But most of what breaks is not aesthetic: it's
timing, parsing, determinism, and regressions in the render. Those are all
testable, and this document is about testing them rather than pretending the
rest is covered.

## Layers

### Unit — `pv-core`, `pv-midi`, `pv-design`

Fast, no GPU, no audio device. This is where most tests live, which is the
main reason the [crate split](02-architecture.md#workspace-layout) puts logic
in GPU-free crates.

- **Tempo map**: tick→second conversion against hand-computed values;
  multiple tempo changes; SMPTE division; degenerate single-tick segments
- **Note normalization**: every row of the [edge-case
  table](03-midi.md#edge-cases) is a test case. Velocity-0 note-on, missing
  note-off, overlapping same pitch, zero duration, pedal extension
- **Visible-window query**: the scan-back correctly includes a long note
  whose start is off-screen — the case that regresses silently and shows up
  as disappearing pedal notes
- **Design parsing**: round-trip, forward-compat with unknown keys, migration
  chain, rejection of `../` in asset paths

### Golden images — `pv-render`

The regression net for visual changes. Render a fixed set of frames from
fixed inputs, compare against committed references.

- Runs headless through `pv-cli` with a software adapter (`lavapipe`), so it
  works in CI without a GPU
- Compared with a perceptual metric and a tolerance, not byte equality —
  different adapters differ in the last bit of floating point, and a
  byte-exact test would fail constantly for no real reason
- Failures write a side-by-side diff image as a CI artifact, because "the
  image changed" is useless without seeing how
- Covers: each shipped Design, empty timeline, single note, dense chord,
  sustain pedal held, extreme zoom, alpha output

Golden images are regenerated deliberately, with the diff reviewed in the PR.
A silently-regenerated golden test is worse than no test, so regeneration is
an explicit command and never automatic.

### Determinism

The properties that [Export](08-export.md) promises, asserted directly:

1. **Frame reproducibility** — render frame `n` twice, expect byte equality.
   Same device, same process; this must hold exactly.
2. **Preview/export parity** — drive the renderer with an `AudioClock` fixed
   at `t` and with a `FrameClock` landing on `t`; the outputs must match
   within tolerance.
3. **Seek independence** — render frame 5000 directly, and render it after
   playing from frame 0. Identical. This is what proves the stateless
   particle design actually delivers what it claims.
4. **Framerate independence** — the same moment rendered at 30fps and 60fps
   produces the same image.

If any of these fail, a core guarantee is broken, and they're cheap enough to
run on every commit.

### Audio

- Offline-render a short MIDI, checksum the output, compare to a reference
- Sequencer event ordering: sample-accurate placement, correct behavior
  across a buffer boundary, note-off before note-on at the same sample
- Seek correctness: no stuck notes after a seek mid-chord — the most common
  real bug in this class of software
- No allocation in the audio callback, verified by a counting allocator

### Fuzzing

MIDI parsing is the attack surface: users open files from strangers.

- `cargo-fuzz` on the MIDI parser, seeded with the corpus
- Property: never panic, never hang, never allocate unboundedly on any input
- Same for the Design parser, which also reads untrusted files
- Run continuously, not just in CI, with findings added to the corpus

### Integration

- Load → play → seek → export a short file end to end
- Export with a stubbed ffmpeg, asserting the arguments and the piped bytes
- Design hot-reload during playback doesn't drop frames or move the playhead
- Device-loss recovery: force a device loss, assert the session survives

## The MIDI corpus

Each case is built in memory by the test itself, using `pv_midi::write` — a
small SMF writer that can also emit the malformed files real software
produces (missing End of Track, truncation, orphan note-offs). Keeping them as
code rather than committed binaries means every fixture is readable in the
test that uses it. What they exercise:

| File | Exercises |
|---|---|
| Case | Exercises |
|---|---|
| format 1 with conductor track | The happy path; tempo and title from track 0 |
| format 0 | Single track, multiple channels, split into tracks |
| format 2 | Independent sequences, concatenated |
| SMPTE division | Negative division / absolute timing |
| tempo changes | Many changes, accumulated correctly |
| pedal | CC64 extension, retrigger cut-off, other-channel pedal |
| retrigger | Same pitch struck again before release |
| stuck notes | Note-ons with no note-off |
| empty | Valid header, no notes |
| truncated / no End of Track | Cut off mid-event |
| 500k notes | Load-time budget, release builds only |
| the demo piece | Round-trip through the writer and back |

## CI

On every push:

1. `cargo fmt --check`, `cargo clippy -- -D warnings`
2. Unit tests on Linux, macOS, Windows
3. Golden images on Linux with `lavapipe`
4. Determinism suite
5. Benchmarks against committed baselines, failing on regression
6. Binary size check against the 20 MB budget, failing on regression
7. `cargo deny` — license compatibility and advisories

Steps 5, 6, and 7 exist because the three stated principles decay quietly
otherwise. A performance or size budget that isn't enforced by CI is an
aspiration, and license compatibility is the one that's genuinely expensive
to discover late.

## What isn't tested automatically

Stated plainly, because pretending otherwise is how gaps hide:

- Whether it looks good. Golden images catch *changes*, not ugliness.
- Audio/video sync as perceived. The calibration screen exists because we
  can't measure display latency; sync is verified by a human with a
  metronome.
- Live-mode latency end to end. Requires real hardware; measured manually
  against a documented procedure at release time.
- Feel — whether scrubbing is responsive, whether defaults are pleasant.
  Manual, and the reason a release checklist exists.
