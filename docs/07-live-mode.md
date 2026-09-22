# 07 — Live mode

Play a connected MIDI keyboard and the visuals react. Embers had this, and
it's the feature that turns the app from a renderer into something you sit
down at.

## Input

[`midir`](https://docs.rs/midir) — cross-platform MIDI input over
CoreMIDI/ALSA/WinMM, with device hotplug.

The input callback runs on the OS MIDI thread and is subject to the same
discipline as the audio callback: it timestamps the event and pushes it into
a lock-free queue. Nothing else. No synthesis, no allocation, no touching
app state.

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

There is also a user-facing offset on top, because perceived sync varies with
display latency, which we can't measure. A calibration screen — a metronome
with a flashing marker and a slider until they line up — is a better answer
than asking someone to guess a millisecond value.

## Visual treatment

Live mode inverts the geometry. There is no future to show, so notes can't
fall from above:

- Notes **rise from the keyboard**, growing upward while the key is held and
  detaching on release to drift up and fade
- Particles fire on press, driven by velocity
- The keyboard lights as in playback
- Optionally, a loaded MIDI still falls from above while the player plays
  along — a practice/duet mode, and the natural way to build a
  play-along video

Because a live note's length isn't known until release, its geometry is
computed from `now - press_time` each frame rather than from a stored
duration. Live notes are the one thing in the renderer that *is* stateful,
which is unavoidable: the future genuinely isn't known. They live in a small
fixed 128-entry array — one slot per possible pitch — so it's still
allocation-free.

## Recording

Live input can be captured to a `NoteTable` and then treated exactly like a
loaded MIDI: scrub it, restyle it, export it. Same structure, same code path,
so recording costs almost nothing to implement beyond the capture buffer, and
exporting a recording needs no new work at all.

Also exportable as a standard `.mid`, so it's usable elsewhere.

## Device handling

- Enumerate on startup, refresh on hotplug
- Remember the last-used device and reconnect automatically
- Multiple simultaneous inputs merged into one stream
- Channel filter, for controllers that split zones across channels
- Velocity curve (linear / soft / hard / custom) — cheap to offer, and
  meaningful on controllers with poorly calibrated action
- Sustain pedal (CC64) honored for both audio and visuals, as in playback
