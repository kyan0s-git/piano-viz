# 07 — Live mode

Play a connected MIDI keyboard and the visuals react. Embers had this, and
it's the feature that turns the app from a renderer into something you sit
down at.

## Input

[`midir`](https://docs.rs/midir) — cross-platform MIDI input over
CoreMIDI/ALSA/WinMM, with device hotplug.

The input callback runs on the OS MIDI thread and is subject to the same
discipline as the audio callback: it timestamps the event and pushes it into
lock-free queues. Nothing else. No synthesis, no allocation, no touching
app state.

The audio queue goes **straight to the audio callback** (`Engine::live_sender`),
never through the UI thread — which would add up to a frame, ~16 ms, of
latency before the synth heard about the key.

No MIDI keyboard? The **computer keyboard** plays too, in the common
"musical typing" layout (home row white keys, the row above black keys, Z/X
for octave, Space as the sustain pedal), and so does **clicking the keys** in
the preview. Those go through the UI thread, so they carry that frame of
latency; fine for sketching, and the reason a MIDI keyboard takes the direct
path.

```
MIDI device
   └─ midir callback ─► timestamp ─► SPSC ring ─┬─► audio thread (synth)
                                                └─► render thread (visuals)
```

Two consumers, two queues, so neither can stall the other.

## Latency budget

The target is under 20 ms from key press to sound, which is around the
threshold where a player stops perceiving the instrument as responsive.
Above roughly 30 ms it feels sluggish to anyone with real keyboard technique.

| Stage | Budget |
|---|---|
| USB MIDI transport | 1-3 ms |
| OS driver to `midir` callback | 1-2 ms |
| Queue to audio callback | < 1 ms |
| Audio buffer | 5.3 ms (256 frames @ 48 kHz) |
| Device output | 2-5 ms |
| **Total** | **~10-16 ms** |

Live mode drops the audio buffer to 256 frames, half the playback default.
That doubles the underrun risk, which is the correct trade when someone is
playing: an occasional click is less damaging than latency that makes the
instrument unplayable. It's exposed as a setting, with the tradeoff stated
plainly rather than hidden behind "performance mode."

## Visual timing

The subtle part. Audio comes out one buffer *after* the synth generates it,
so drawing a key-press the instant the event arrives puts the visual ahead of
the sound. It looks fine in isolation and wrong side-by-side — the flash
precedes the note.

So visuals are delayed by the measured output latency, to land together:

```rust
let visual_time = event.timestamp + output_latency;
```

Perceived sync also varies with display latency, which we can't measure. A
calibration screen — a metronome with a flashing marker and a slider until
they line up — is the planned answer. **Not built yet.**

## Visual treatment

Live mode inverts the geometry. There is no future to show, so notes can't
fall from above:

- Notes **rise from the keyboard**, growing upward while the key is held and
  detaching on release to drift up and fade
- Particles fire on press, driven by velocity
- The keyboard lights as in playback
- Play-along — a loaded MIDI falling from above while the player plays —
  is planned but **not built yet**: it needs two note sets drawn in
  opposite directions in one frame

Because a live note's length isn't known until release, a held note's
duration is `now - press_time`, recomputed each frame. The recorder keeps
held keys in a fixed 128-slot array and finished notes in a list, and each
frame hands the renderer a small score of what's on screen, written into the
existing note buffer. That per-frame score is a small allocation, live mode
only; playback and export stay allocation-free. Notes released under the
sustain pedal keep ringing until it lifts or the key is struck again.

## Recording

Live input can be captured to a `NoteTable` and then treated exactly like a
loaded MIDI: scrub it, restyle it, export it. Same structure, same code path,
so recording costs almost nothing to implement beyond the capture buffer, and
exporting a recording needs no new work at all.

Also exportable as a standard `.mid`, so it's usable elsewhere.

## Device handling

Built:

- Enumerate on startup, with a refresh button
- All channels of a device merged into one stream
- Sustain pedal (CC64) honored for both audio and visuals, as in playback
- Clear status when MIDI input isn't available at all
- An optional 256-frame low-latency audio buffer

Planned, not built yet:

- Automatic hotplug detection and reconnecting to the last-used device
- Several devices at once
- Channel filter, for controllers that split zones across channels
- Velocity curves (soft / hard / custom), for controllers with poorly
  calibrated action
