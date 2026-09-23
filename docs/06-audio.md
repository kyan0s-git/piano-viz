# 06 — Audio

## Synthesis

[`rustysynth`](https://docs.rs/rustysynth) — a pure-Rust SF2 synthesizer,
MIT-licensed, no C dependency, and usable for both real-time and offline
rendering. That last property matters: the same synth renders the preview and
the export, so exported audio matches what was previewed.

FluidSynth would sound marginally better but is LGPL, which conflicts with
shipping a permissively-licensed static binary. `oxisynth` is the other pure-
Rust option; `rustysynth` has broader adoption and fewer dependencies, which
fits the "small" principle better.

### The built-in piano

**`rustysynth` reads SF2 only — not SF3**, so a compressed bundled font
wasn't available. Rather than ship an 8 MB SF2, the built-in piano is
*synthesized at startup* (`pv-audio/src/piano.rs`) and packed into an
in-memory SF2 that the same synthesizer plays. That puts zero bytes of audio
in the binary and leaves no licensing question.

The model is additive and physically motivated: stiff-string inharmonicity
rising toward the treble, a hammer-position notch in the spectrum, two-stage
("prompt" and "aftersound") decay with higher partials dying faster, up to
three detuned unison strings whose beating is much of a piano's character, a
filtered-noise hammer transient, and two velocity layers since rustysynth has
no velocity-to-filter modulation. Samples sit every six semitones. Generation
takes about 0.75 s on four cores and runs off the UI thread.

It's a respectable default, not a concert grand. Loading any SF2 replaces it,
and that's the real answer for quality: FluidR3_GM (MIT) and GeneralUser GS
are good freely-licensed choices.

### Loudness

The synth runs at 3x rustysynth's default gain, which puts typical piano
music around -6 dBFS, followed by a stateless soft limiter (transparent below
0.8, tanh knee above) so a dense fortissimo chord saturates gently instead of
clipping. Being stateless, it keeps offline renders bit-reproducible.

## Output

[`cpal`](https://docs.rs/cpal) for cross-platform output — WASAPI, CoreAudio,
ALSA/JACK. Buffer size is configurable; default 512 frames at 48 kHz, about
10.7 ms, which is a reasonable compromise between latency and underrun safety
on typical consumer hardware. Live mode lowers it — see
[Live mode](07-live-mode.md).

## The audio thread contract

The callback runs on a real-time deadline. Miss it and the user hears a
click, which is far more noticeable than a dropped video frame. So the
callback may not:

- allocate or free
- take a lock
- perform I/O of any kind, including logging
- call anything that might do the above internally

Everything it needs is preallocated at load. It communicates outward through
atomics and inward through a lock-free SPSC ring buffer (`rtrb`):

```rust
/// Main thread -> audio thread. Never blocks either side.
enum AudioCommand {
    Seek { sample: u64 },
    Play, Pause,
    SetVolume(f32),
    SetTrackMute(TrackMask),
    NoteOn { pitch: u8, velocity: u8 },   // live mode
    NoteOff { pitch: u8 },
}
```

Commands that would allocate — loading a different SoundFont, replacing the
score — are handled by preparing the new object on another thread and
sending it over, so the audio thread only moves a pointer. The object it
replaces is sent back through a second ring buffer and dropped on the main
thread, never in the callback. Seeks carry their controller "chase" state
(pedal, program, bend in effect at the seek point), computed off the audio
thread, so jumping into a pedaled passage sounds right.

## Sequencing

The audio thread owns the playhead and emits note events inside the
callback's buffer rather than at its boundary. Quantizing note starts to the
512-sample buffer would put notes up to 10 ms early or late, audible on fast
passages as a loss of crispness. Events land on the synth's internal 64-sample
block instead — 1.3 ms at 48 kHz, well below audibility.

Within a callback of `n` frames:

```
for each note event in [playhead, playhead + n):
    render audio up to the event's exact sample offset
    apply the event to the synth
render the remainder
publish playhead + n  (atomic, Release ordering)
```

The note events come from a forward cursor into the `NoteTable`, which is
sorted by start time — so this is a pointer walk, not a search. Seeking
resets the cursor via binary search, which happens on the main thread and
arrives as a command.

## Clock publication

```rust
// Audio thread, end of callback:
playhead_samples.store(new_position, Ordering::Release);

// Render thread, start of frame:
let t = playhead_samples.load(Ordering::Acquire) as f64 / sample_rate as f64;
```

Release/Acquire rather than Relaxed so that any state the audio thread wrote
before publishing — the current key-press array, for instance — is visible to
the renderer that reads the new playhead. With Relaxed the renderer could see
a new playhead alongside stale key states, which would show up as keys
lighting a frame late, intermittently.

This is also where the audio/video sync guarantee lives: the renderer draws
whatever the audio thread says has played. It cannot drift, because there is
only one source of truth.

### Output latency compensation

`cpal` reports the stream's output latency. Sound the user hears at wall time
`T` was generated by the callback some milliseconds earlier. To make the
visuals line up with what's *heard* rather than what's been *computed*, the
renderer subtracts that latency from the playhead before drawing. Without
this correction the visuals run consistently ahead of the audio by the buffer
size — small, but on percussive attacks it's perceptible, and it's the kind
of wrongness people feel without being able to name.

## Offline rendering for export

Export doesn't use `cpal` at all. The synth runs as fast as the CPU allows,
writing to a WAV that the muxer picks up:

- Render in blocks, applying the same sample-accurate sequencing
- Deterministic: same input, same bytes out, every time
- Typically 50-100x faster than real time, so audio is never the bottleneck
- 32-bit float internally; dithered to 16-bit or kept as float depending on
  the output container

Because the sequencing logic is shared with the real-time path rather than
reimplemented, exported audio and previewed audio can't drift apart — the
usual failure mode where an export subtly differs from what was auditioned.

## Metronome and lead-in

Optional click track, synthesized directly rather than sampled. Useful for
live mode and for verifying sync. Excluded from export by default, since
almost nobody wants a metronome in the final video, but available as a
toggle.
