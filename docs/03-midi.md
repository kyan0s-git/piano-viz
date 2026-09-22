# 03 — MIDI pipeline

Parsing MIDI is deceptively easy to get 90% right and surprisingly annoying
to get fully right. Most of this document is the last 10%, because that's
where visualizers break on real files.

## Library

[`midly`](https://docs.rs/midly) — zero-copy, no-panic, handles all three SMF
formats and running status, and is fast enough that a 500k-note file parses
in well under a second. We wrap it rather than expose it, so the rest of the
app never sees raw MIDI events.

## The note model

Notes are packed to 16 bytes so a million of them cost 16 MB and a cache line
holds four:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Note {
    pub start: f32,     // seconds from song start
    pub duration: f32,  // seconds; always > 0 after normalization
    pub pitch: u8,      // 0..=127
    pub velocity: u8,   // 1..=127
    pub track: u8,      // index into TrackTable, saturating at 255
    pub flags: u8,      // bit 0: sustained by pedal, bit 1: is_black_key
    pub _pad: [u8; 4],
}
```

`#[repr(C)]` + `Pod` means the CPU-side array uploads to a GPU storage buffer
with a straight `bytemuck::cast_slice` — no conversion pass, no shadow copy.

Seconds rather than ticks, resolved once at load, because every consumer
(renderer, sequencer, export) wants seconds and resolving tempo per-access
would be both slow and repetitive.

`f32` gives ~7 significant digits: at 3600 seconds the granularity is about
0.25 ms, well under a sample at 48 kHz being audible or a frame being
visible. Good enough for any realistic piece, and halving the struct size
matters more.

### NoteTable

```rust
pub struct NoteTable {
    notes: Vec<Note>,           // sorted by (start, pitch)
    max_duration: f32,          // for the visible-window scan-back
    per_pitch: [Vec<u32>; 128], // indices, for live highlighting
}
```

Sorted by start time because the renderer's visible-window query is a binary
search, and the sequencer's playback is a forward scan. `max_duration` is
cached because the window query needs it — see
[Rendering](04-rendering.md#visible-window-query).

## Tempo map

Tempo changes are cumulative, so converting a tick to seconds means summing
every tempo segment before it. Doing that per note is O(n·m). Instead we
build the segment table once and binary search it:

```rust
struct TempoSegment {
    tick: u32,          // where this tempo starts
    seconds: f64,       // elapsed seconds at that tick
    us_per_quarter: u32,
}
```

`seconds` is precomputed by walking the segments once, so a tick→seconds
conversion is a binary search plus one multiply. `f64` here, not `f32`: this
is accumulated arithmetic where error compounds, unlike the final note times.

### Division modes

The SMF header division field has two meanings depending on its sign bit, and
the second one gets forgotten:

- **Metrical** (positive): ticks per quarter note. Tempo events apply.
  `seconds = ticks / tpqn * us_per_quarter / 1e6`
- **SMPTE** (negative): frames per second and ticks per frame, giving an
  absolute time base. **Tempo events are ignored entirely** — the timing is
  already in real time. Rare, but files that use it play at wildly wrong
  speed if you treat the division as metrical.

## Edge cases

Each of these is a real thing that appears in files people actually have, and
each has a defined behavior rather than a crash:

| Case | Behavior |
|---|---|
| Format 0 (one track, many channels) | Split into virtual tracks by channel |
| Format 1 (simultaneous tracks) | Tracks as authored; tempo map from track 0 |
| Format 2 (independent sequences) | Concatenate, with a warning — genuinely ambiguous |
| Note-on with velocity 0 | Treat as note-off. Extremely common; not optional |
| Note-on over an already-sounding same pitch | Close the previous, start a new one. Matches piano behavior |
| Note-off with no matching note-on | Discard silently |
| Note-on with no note-off before EOF | Clamp to end of track, flag it |
| Zero-duration note | Clamp to a 1 ms floor so it's visible and audible |
| Sustain pedal (CC64) | Extend visually and audibly to pedal release; set `flags` bit 0 |
| Sostenuto (CC66) / soft (CC67) | Passed to the synth; sostenuto not visualized in v1 |
| Channel 10 (percussion) | Excluded from the keyboard, kept for audio |
| Pitch bend | Sent to the synth; no visual in v1 |
| Track with no notes | Kept in the track list so its name shows, but hidden |
| Truncated / malformed chunk | Parse what's valid, report the rest as a warning |
| Empty file / no notes at all | Load successfully, show an empty timeline |

The sustain pedal one matters most visually. Without it, pieces with heavy
pedaling look staccato and wrong while sounding sustained — the visual and
the audio disagree, which reads as a bug even to someone who can't say why.

## Track metadata

Extracted for the track list and for text overlays:

- Track name (meta 0x03), instrument name (0x04), copyright (0x02)
- Program change → a General MIDI instrument name
- Channel, note count, pitch range, time range

## Auto-detection

Run once at load. Every result is a starting suggestion the user can
override, never a permanent decision:

- **Key range** — the actual min/max pitch, so a piece that never leaves two
  octaves can render a two-octave keyboard instead of a mostly-idle 88.
- **Hand split** — if exactly two tracks, treat them as left/right. Otherwise
  find the pitch gap with the cleanest separation, defaulting near C4.
- **Track colors** — assign from the Design's palette in track order.
- **Lead-in** — if the first note is more than two seconds in, offer to trim.

## Performance

Target: under one second to load a 500k-note file, on one thread.

- Parse with `midly`'s zero-copy borrowing — no intermediate `Vec<Event>`
- Pre-size the note vector from a cheap event count so it never reallocates
- Sort with `sort_unstable_by_key` on a packed `u64` of `(start_bits, pitch)`
- Build the per-pitch index in the same pass as the sort's output

Parsing happens on a worker thread with a progress channel, so a huge file
shows a progress bar rather than freezing the window.
