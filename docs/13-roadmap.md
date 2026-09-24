# 13 — Roadmap

Sequenced so that something runs end to end early and stays running. The
ordering principle: build the spine first — MIDI in, pixels out, one file
exported — then add the flesh. A half-built pipeline that already exports
something is far easier to reason about than three finished subsystems that
have never been connected.

Scope is full Embers parity including live mode, so live mode is a milestone
rather than a deferred nice-to-have.

## Status

| Milestone | State |
|---|---|
| M0 Foundations | Done |
| M1 MIDI in, notes on screen | Done |
| M2 Sound and time | Done |
| M3 The look | Done |
| M4 Designs | Done — zipped `.pvd` bundles deferred |
| M5 Export | Done |
| M6 Live mode | Done — play-along, latency calibration, hotplug, velocity curves not yet |
| M7 Polish and release | Not started — see below |

Measured against the [success criteria](01-vision.md#success-criteria) so
far: load of 500k notes in ~93 ms (budget 1 s); bit-identical re-renders
and exported frames byte-identical to the preview (tested); app idles at 0%
CPU when paused; binaries 12 MB (app) and 5.3 MB (CLI) against a 20 MB
budget. **Not yet measured on real hardware**: 1080p60 preview on
integrated graphics and the 90-second export budget — development ran on a
software rasterizer, where a 10 s 1080p60 export took about two minutes.

## M0 — Foundations

Workspace, CI, licenses, the shape of the thing.

- Cargo workspace with all eight crates, empty but wired
- `pv-core` types: `Note`, `NoteTable`, `Clock`, `Score`, `Design`, `Session`
- CI: fmt, clippy, test, `cargo deny`
- Both license files, README, this documentation

**Done when:** `cargo test` passes on all three platforms in CI.

## M1 — MIDI in, notes on screen

The spine. Not pretty, but every layer is present.

- `pv-midi`: parse → `NoteTable`, tempo map, normalization
- The full [edge-case table](03-midi.md#edge-cases), with tests
- `pv-render`: wgpu init, HDR target, instanced note quads with SDF corners
- Keyboard geometry with correct black-key offsets
- `pv-app`: window, preview viewport, a hardcoded clock
- Visible-window query

**Done when:** a MIDI file renders as falling notes above a keyboard,
scrubbable by a slider.

## M2 — Sound and time

Now it's a player.

- `pv-audio`: `rustysynth`, `cpal`, the audio thread contract
- Sample-accurate sequencing; the playhead atomic
- `AudioClock`; output-latency compensation
- Transport: play/pause/seek/loop/speed
- Key-press state driving the keyboard visuals

**Done when:** a MIDI plays in sync with the visuals and seeks cleanly with
no stuck notes.

## M3 — The look

The milestone where it starts resembling Embers.

- Bloom chain, tonemapping, vignette, grain, dither
- [Stateless particles](05-particles.md): `NoteHit` and `NoteHold` emitters
- Backgrounds: solid, gradient, image; reflection
- Camera: analytic shake and drift
- Note styling: gradients, borders, HDR intensity
- Color sources: track, channel, hand, pitch class, velocity

**Done when:** the default output is something a user would post without
touching a setting.

## M4 — Designs

Making the look ownable and shareable.

- `pv-design`: TOML schema, serde, validation, migration chain
- `.pvd` bundles, asset resolution with path-escape rejection
- Hot reload via `notify`
- The five shipped Designs
- Design inspector panel with live binding, undo/redo

**Done when:** a Design can be edited in a text editor and the preview
updates mid-playback, and Designs can be swapped from the UI.

## M5 — Export

The second half of the brief.

- `pv-export`: frame pump, `FrameClock`, three-buffer readback ring
- Row-padding handling
- ffmpeg subprocess piping; detection with a clear failure path
- PNG-sequence fallback
- Offline audio render and mux
- Alpha export (ProRes 4444, VP9)
- Render dialog: progress, live thumbnail, ETA, cancel
- `pv-cli`: `render`, `batch`, `inspect`
- The [determinism suite](12-testing.md#determinism) and golden images

**Done when:** a 3-minute piece exports to MP4 in under 90 seconds, and the
determinism tests pass.

## M6 — Live mode

- `midir` input, device enumeration, hotplug
- Low-latency path: 256-frame buffer, lock-free queues to synth and renderer
- Rising-note visuals for live input
- Latency calibration screen
- Recording to `NoteTable`, exportable as `.mid` or straight to video
- Play-along: a loaded MIDI falling while the player plays

**Done when:** playing a connected keyboard produces sound and visuals under
20 ms, measured.

## M7 — Polish and release

- First-run experience: demo piece loaded and playing
- Timeline density strip, bar/beat markers
- Track panel: reorder, solo, bulk palette assignment
- Parameter search
- Accessibility: keyboard navigation, reduced-motion, DPI scaling
- Performance HUD, benchmark baselines, binary-size enforcement
- Packaging: MSI/NSIS, signed `.app` in a DMG, AppImage and Flatpak
- User documentation and a Design-authoring guide

**Done when:** the [success criteria](01-vision.md#success-criteria) are all
met and verified.

## Beyond v1

Deferred deliberately, not forgotten:

- Video backgrounds
- User-authored shaders for backgrounds and effects
- Multi-instrument layouts — the `InstrumentLayout` seam already exists
- MIDI-file light editing: quantize, transpose, time shift
- A Design browser with in-app sharing
- Distributed or cloud rendering
- Live-streaming output (virtual camera / NDI)

## Risks

The things most likely to go wrong, and what we do about them:

| Risk | Mitigation |
|---|---|
| Visual quality falls short of Embers | M3 is a checkpoint against reference footage; adjust before building further on it |
| `wgpu` backend inconsistencies across platforms | CI on all three from M0; golden images on software adapter |
| Stateless particles prove too limiting | The stateful-emitter escape hatch is designed for in [Particles](05-particles.md) |
| Bundled SoundFont too large or poor | SF2-only is a known constraint; user-supplied SF2 is the quality answer |
| ffmpeg absent for users | Detected up front, PNG fallback always works |
| Binary size creeps past 20 MB | CI enforces it from M0, so it's a build failure rather than a late discovery |
| Audio thread priority on Linux | Document PipeWire/JACK setup; degrade to a larger buffer rather than glitching |
