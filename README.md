# piano-viz

A piano MIDI visualizer: load a MIDI file, get falling notes, a lit keyboard,
particles and glow — previewed in real time and exported to video.

Inspired by [Embers](https://embers.app/) by LyricWulf, which stopped
development. This is an open-source, cross-platform successor.

> **Status: working, pre-release.** Milestones M0–M6 are implemented:
> playback, the full look, Designs, video export, the CLI, and live mode.
> Packaging and polish (M7) remain. See [`docs/13-roadmap.md`](docs/13-roadmap.md).

## What it does

- Load a piano MIDI file, watch it play back with Embers-class visuals
- Tune every visual parameter live and see the result immediately
- Export to video at any resolution/framerate, with alpha for compositing
- Play your own MIDI keyboard and have the visuals react in real time

## Why another one

| | Embers | Synthesia | SeeMusic | piano-viz |
|---|---|---|---|---|
| Platforms | Windows | Win/Mac | Win/Mac | Win/Mac/Linux |
| Open source | No | No | No | Yes |
| Render export | Paid tier | Limited | Paid tier | Yes |
| Alpha export | No | No | No | Yes |
| Headless CLI | No | No | No | Yes |
| Text-editable themes | No | No | No | Yes |

The gaps worth filling: Embers was Windows-only and closed, so when
development stopped the work stopped with it. Nothing in that feature set
requires being closed or Windows-bound.

## Build and run

Needs Rust 1.95+ and, on Linux, ALSA headers (`libasound2-dev`).

```sh
cargo run --release -p pv-app          # the app; opens on a demo piece
cargo run --release -p pv-cli -- render song.mid -o song.mp4
cargo run --release -p pv-cli -- --help
```

Video export uses `ffmpeg` if it's installed; PNG-sequence export needs
nothing extra. Drop a `.mid`, a Design `.toml`, or a SoundFont `.sf2` onto
the window to open it.

Sizes today: the app is a 12 MB binary and the CLI 5.3 MB, with the piano
sound synthesized at startup rather than shipped as samples.

## Design principles

Three constraints, in priority order when they conflict:

1. **Fast** — 1080p60 preview stays at 60fps on integrated graphics. Export
   runs faster than real time.
2. **Efficient** — no per-frame allocation on the hot path. The audio thread
   never locks. Work scales with what's *visible*, not with file size.
3. **Small** — one binary under 20 MB, no runtime to install, dependency
   count kept deliberately low.

None of these come at the cost of how it looks. The visual target is Embers,
which set a high bar.

## Stack

Rust, with [`wgpu`](https://wgpu.rs) for graphics (Vulkan/Metal/D3D12),
[`midly`](https://docs.rs/midly) for MIDI, [`rustysynth`](https://docs.rs/rustysynth)
for SoundFont synthesis, [`cpal`](https://docs.rs/cpal) for audio output, and
`egui` for the interface.

## Documentation

Start at [`docs/README.md`](docs/README.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
