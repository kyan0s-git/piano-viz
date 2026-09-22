# 02 — Architecture

## Workspace layout

A Cargo workspace of small crates with one-directional dependencies. The
point is that `pv-core` has no GPU, audio, or OS dependencies, so the bulk of
the logic compiles and tests in a second without a graphics device.

```
crates/
  pv-core     types, time, note model, project state   deps: serde, glam
  pv-midi     MIDI -> NoteTable                        deps: pv-core, midly
  pv-design   Design schema, parsing, hot-reload       deps: pv-core, serde, toml
  pv-audio    synthesis, output, offline render        deps: pv-core, rustysynth, cpal
  pv-render   wgpu pipelines, layers, post-processing  deps: pv-core, pv-design, wgpu
  pv-export   frame pump, encoder plumbing             deps: pv-render, pv-audio
  pv-app      winit + egui desktop shell               deps: all of the above
  pv-cli      headless renderer                        deps: pv-export, clap
```

```
                pv-core
      ┌────────────┼────────────┬───────────┐
   pv-midi     pv-design     pv-audio       │
      └────────────┼────────────┤           │
                pv-render ──────┘           │
                    │                       │
                pv-export ──────────────────┘
                    │
            ┌───────┴───────┐
         pv-app          pv-cli
```

Why split this way: `pv-render` is the only crate that needs a GPU, and
`pv-app` is the only one that needs a window. That makes `pv-cli` genuinely
headless and makes CI possible without a display server.

## Threads

Four, and no more. Every additional thread is a synchronization bug waiting
to happen.

| Thread | Owns | Must never |
|---|---|---|
| **Main** | winit event loop, egui, wgpu queue submission | block on I/O |
| **Audio** | `cpal` callback, synth, the playhead | allocate, lock, or syscall |
| **Worker pool** | MIDI parsing, SoundFont loading, image decode | touch GPU state |
| **Export** | the frame pump, encoder stdin | run while preview is active |

The audio thread's constraints are the strict ones. It runs on a real-time
deadline — miss it and the user hears a click. So: no `Vec` growth, no
`Mutex`, no file access, no logging. Everything it needs is preallocated
before playback starts, and it communicates through lock-free ring buffers
(`rtrb`) and atomics.

Export and preview are mutually exclusive by design. Sharing the GPU between
a real-time preview and a flat-out export just makes both slow and the
preview stutter; we suspend preview rendering during export and show progress
instead.

## Clocks

This is the decision that prevents audio/video drift, so it's worth being
precise about.

A naive visualizer accumulates `dt` from frame timings to advance the
playhead. It drifts, because frame timing and audio playback rate are
independent and neither is exact. By the three-minute mark the notes don't
line up with the sound, and the error is not recoverable.

Instead, **the audio callback owns the playhead.** It knows exactly how many
samples it has produced, which is the only truly authoritative measure of how
much music has played. It publishes that as an atomic:

```rust
/// Samples played since transport start. Written only by the audio thread.
playhead_samples: AtomicU64,
```

The renderer reads it and converts to seconds. Video follows audio; audio
never waits for video. A dropped frame costs one frame of smoothness and
nothing else — the next frame lands at the correct time rather than
accumulating the error.

Both time sources implement one trait:

```rust
pub trait Clock {
    /// Current playhead position in seconds.
    fn now(&self) -> f64;
}

/// Preview: reads the atomic the audio thread publishes.
pub struct AudioClock { samples: Arc<AtomicU64>, sample_rate: u32 }

/// Export: frame n is exactly n/fps. Rational, so no accumulated float error.
pub struct FrameClock { frame: u64, fps_num: u32, fps_den: u32 }
```

`FrameClock` uses a rational framerate so 29.97 (30000/1001) and 59.94 are
exact rather than approximated. Computing `frame as f64 * (1.0/29.97)` and
accumulating would visibly desync over a long render.

Everything downstream takes a `&dyn Clock` and cannot tell which it has. That
identity is what makes preview and export match.

## Data flow

```
  .mid file
     │  pv-midi (worker thread, once)
     ▼
  NoteTable ──────────────────────────────┐  sorted by start time,
     │                                    │  uploaded to GPU once
     │                                    │
     ├──► Sequencer (audio thread) ──► Synth ──► cpal ──► speakers
     │         emits note on/off              │
     │         at sample accuracy             └──► playhead (atomic)
     │                                                  │
     │                                                  ▼
     └──► Renderer (main thread) ◄──── Clock ───────────┘
                │
                │  binary search visible window
                ▼
          GPU: notes + keyboard + particles + post
                │
                ├──► surface (preview)
                └──► readback ──► encoder (export)
```

The `NoteTable` is uploaded to GPU memory once at load and never re-uploaded.
Per frame the CPU does a binary search to find the visible range and issues a
handful of instanced draws. It does not walk the note list, build vertex
buffers, or allocate. This is why a 500k-note file costs the same per frame
as a 500-note file — see [Performance](11-performance.md).

## State ownership

Three distinct kinds of state, kept separate because they have different
lifetimes and different persistence rules:

```rust
/// The music. Immutable after load.
pub struct Score { notes: NoteTable, tempo: TempoMap, meta: Meta }

/// The look. Hot-reloadable, serialized to .pvd, shared between users.
pub struct Design { background: Background, notes: NoteStyle, /* ... */ }

/// The session. Not part of a Design — belongs to this sitting.
pub struct Session { playhead: f64, loop_range: Option<Range<f64>>,
                     muted_tracks: TrackMask, volume: f32, zoom: f32 }
```

Keeping `Design` and `Session` apart matters for a practical reason: when a
user shares a Design, nobody wants their playhead position, mute states, or
output path coming along with it. A Design is portable; a Session isn't.

`Score` being immutable after load is what lets the renderer and the audio
thread both read it without synchronization — it's behind an `Arc` and
neither writes.

## Error handling

- `thiserror` for typed errors in libraries, `anyhow` for context in the
  binaries. Standard split.
- **No `unwrap()` on anything derived from user input.** A malformed MIDI, a
  truncated SoundFont, or a Design from a future version must produce a
  readable message, not a panic. The MIDI corpus in
  [Testing](12-testing.md) exists to enforce this.
- Design files are **forward-compatible**: unknown keys warn and are ignored
  rather than failing the load, so a Design saved by a newer build still
  mostly works on an older one.
- GPU device loss (driver reset, laptop GPU switch) is recoverable:
  reconstruct the device and re-upload, don't crash. This genuinely happens
  on Windows and losing an unsaved session to it is unacceptable.
