# 11 — Performance

The brief asks for fast, efficient, and small without compromising quality.
Those are measurable, so this document states the numbers and the mechanisms
that hold them.

## Budgets

### Preview — 1080p60 on integrated graphics

16.6 ms per frame total. Target allocation:

| Stage | Budget |
|---|---|
| CPU: visible-window query, uniforms | 0.3 ms |
| GPU: background | 0.2 ms |
| GPU: notes | 1.0 ms |
| GPU: keyboard | 0.3 ms |
| GPU: particles | 1.5 ms |
| GPU: bloom chain | 2.0 ms |
| GPU: tonemap + post | 0.8 ms |
| GPU: egui | 0.7 ms |
| **Total** | **~6.8 ms** |

Roughly 40% utilization, leaving headroom for a busy passage, a slower
adapter, or a 4K preview window. Budgeting to the full frame leaves nothing
for the bad cases, which are the ones users notice.

### Load

| Operation | Target |
|---|---|
| Cold start to window | < 300 ms |
| Parse 10k-note MIDI | < 30 ms |
| Parse 500k-note MIDI | < 1 s |
| Load 8 MB SoundFont | < 200 ms |
| Design hot-reload | < 10 ms |

### Export

3-minute piece, 1080p60, 2x supersampling: under 90 seconds on a mid-range
discrete GPU. That's ~10,800 frames in 90 s, about 8.3 ms per frame including
readback and encode — achievable because the render itself is well under
budget and readback is pipelined.

### Memory

| | Target |
|---|---|
| Idle | < 120 MB |
| 10k-note piece loaded | < 200 MB |
| 500k-note piece loaded | < 300 MB |
| Exporting 4K | < 700 MB |

### Binary

Under 20 MB including the bundled SoundFont — so under 12 MB of code, which
`wgpu` plus `egui` plus `rustysynth` fits within after LTO and stripping.

## What makes it fast

### Work scales with what's visible

The central property. A 500k-note file and a 500-note file cost the same per
frame, because per frame we touch only the notes in the visible window.

- Notes: two binary searches, one instanced draw over a contiguous range
- Particles: derived from the visible note range, no buffer, no simulation
- Keyboard: fixed 128 entries regardless of anything

There is no path where per-frame cost is proportional to file size. That's a
structural property of the design, not an optimization to be maintained.

### No per-frame allocation

Every buffer is allocated at load or at resize. The render loop allocates
nothing; `Vec`s that are reused are cleared, not dropped. The audio thread's
constraint is stricter still — see [Audio](06-audio.md#the-audio-thread-contract).

This is enforced, not just intended: a debug allocator counts allocations
during a frame and the test suite fails on any nonzero count in the hot path.

### Upload once

The `NoteTable` goes to GPU memory at load and is never re-uploaded. Per
frame the only host→device traffic is one small uniform block: time, camera,
and Design parameters. A few hundred bytes.

### One draw call per layer

Instancing everywhere. Draw-call count is constant in note density.

### Pipelined readback

Three staging buffers so CPU and GPU never wait on each other during export —
see [Export](08-export.md#why-a-ring-of-three).

## What makes it small

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

`panic = "abort"` removes unwinding tables, which is a meaningful fraction of
a Rust binary. It's safe here because there is nothing to recover to — a
panic in this app is a bug, not a condition to handle.

Dependency discipline:

- No async runtime. There's no concurrency problem here that needs one; a
  thread and a channel are smaller and clearer.
- No `image` with all features — only the decoders actually used
- No `regex`, no `chrono`, no heavyweight serialization beyond `serde` + `toml`
- Prefer a hundred lines written over a dependency pulling twenty crates

Dependency count is tracked in CI, and additions are justified in review. The
practical enemy of a small Rust binary is not any single large crate, it's
dozens of small ones arriving as transitive dependencies of convenience.

## Scaling limits

Black MIDI is the stress case, and the honest answer has a ceiling:

| Notes | Behavior |
|---|---|
| < 100k | Everything at full quality |
| 100k-1M | Full quality; load takes a second |
| 1M-5M | Fine, but particles should be reduced — a warning offers to |
| > 5M | Auto-degrade: cap particles, simplify note styling, warn clearly |

Degradation is explicit and announced, never silent. Someone rendering a
black MIDI should be told their particle settings were capped, not left to
wonder why it doesn't match the preview.

## Measurement

Performance claims that aren't measured decay, so:

- `F3` HUD with per-stage GPU timings from timestamp queries
- `criterion` benchmarks on MIDI parsing, the window query, and the tempo map
- GPU captures via RenderDoc when something looks wrong
- A CI benchmark job on a fixed runner, comparing against recorded baselines
  and failing on regressions beyond a threshold

Baselines are committed, so a regression is a diff rather than a memory.
