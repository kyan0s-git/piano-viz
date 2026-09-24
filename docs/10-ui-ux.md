# 10 — UI & UX

## Toolkit

`egui`, rendered through the same `wgpu` device as the scene. One device, one
frame, no compositing seam and no second graphics stack in the binary.

`egui` is not the prettiest toolkit available, and it's a deliberate trade:
it's small, it has no platform dependencies, it draws through our existing
device, and it's immediate-mode, which suits a UI whose panels are almost
entirely sliders bound to live state. A retained-mode toolkit would mean
keeping widget state in sync with Design state, which is exactly the busywork
immediate mode removes.

## Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│  File   Edit   View   Render   Help                              ─ □ ✕│
├────────────┬──────────────────────────────────────────┬──────────────┤
│            │                                          │              │
│  TRACKS    │                                          │   DESIGN     │
│            │                                          │              │
│ ● Right H. │                                          │ ▸ Background │
│ ● Left H.  │              PREVIEW                     │ ▾ Notes      │
│ ○ Perc.    │                                          │   Radius  ▬▬ │
│            │                                          │   Border  ▬  │
│  ────────  │                                          │   Glow    ▬▬▬│
│  Design    │                                          │ ▸ Keyboard   │
│  Ember ▾   │                                          │ ▸ Particles  │
│            │                                          │ ▸ Post       │
│            │                                          │ ▸ Camera     │
├────────────┴──────────────────────────────────────────┴──────────────┤
│  ▶  ⏮ ⏭   0:42 / 3:15  ├──────●────────────────────┤  🔁  1.0x  🔊  │
└──────────────────────────────────────────────────────────────────────┘
```

Side panels collapse; the preview is the point and should be able to fill the
window. `View → Presentation` gives a borderless fullscreen preview, which is
what you want when showing someone the result or capturing with OBS.

## Preview viewport

- Renders at the Design's aspect ratio with letterboxing, so what's on screen
  is framed exactly as the export will be. A preview that fills a differently-
  shaped window is actively misleading about composition.
- Optional safe-area guides for title/action, for anyone composing overlays
- Performance HUD (`F3`): fps, frame time, visible notes, live particles,
  GPU memory
- Scroll to zoom, middle-drag to pan, `0` to reset

## Timeline

More than a progress bar, because scrubbing is how people navigate:

- A density strip showing note activity over the whole piece — this is what
  makes a section findable at a glance rather than by hunting
- Drag to scrub, with the visuals updating live. [Stateless
  particles](05-particles.md) are what make this instant rather than a stall
- Loop region: shift-drag on the timeline, or `[` and `]` to set it from the
  playhead
- Bar/beat markers from the tempo map
- Click a track's row to solo it temporarily

## Track panel

Per track: color swatch (click to override, right-click the name to reset),
a `notes` toggle for visibility, a `sound` toggle for mute, and the name —
click it to solo. Visibility and sound are independent: hide a hand's notes
and still hear it, or the reverse. (Reordering tracks' draw order is still
to do.)

Bulk actions: assign a palette across all tracks, reset overrides, auto-split
hands.

## Design inspector

Collapsible sections mirroring the [Design schema](09-design-format.md), so
the GUI and the file format teach each other. Someone who learns the panel
can read the TOML, and vice versa.

- Sliders with numeric entry — drag for feel, type for precision
- Color pickers with an eyedropper and recent swatches
- Curve editors for size/opacity envelopes
- Every control has a reset-to-default affordance
- Changes apply live; no Apply button
- Undo/redo across all Design edits (`Ctrl+Z` / `Ctrl+Shift+Z`)

A search field over all parameters. With this many controls, remembering
which section holds "chromatic aberration" is a real cost, and search is much
cheaper to build than a perfect taxonomy.

## Preview

Click the picture to play or pause; double-click for presentation mode.
While nothing moves — paused, no animation, no input — the app stops
redrawing entirely and idles at 0% CPU.

## Render dialog

Range, resolution, framerate, preset, supersampling, output path, estimated
file size. Then progress with a live thumbnail, ETA, and cancel.

The ffmpeg check runs when the dialog opens, not when rendering starts — so a
missing encoder is discovered immediately, with the PNG-sequence fallback
offered right there rather than after a wasted wait.

## Keyboard shortcuts

Conventional where conventions exist; video-editor conventions where they
apply, since that's the neighboring tool for this audience.

| Key | Action |
|---|---|
| `Space` | Play / pause |
| `J` `K` `L` | Rewind / stop / forward (shuttle) |
| `←` `→` | Step one beat (`Shift` for a bar) |
| `Home` `End` | Start / end |
| `[` `]` | Set loop in / out |
| `Ctrl+L` | Toggle loop |
| `Ctrl+O` | Open MIDI |
| `Ctrl+R` | Render |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `F3` | Performance HUD |
| `F11` | Presentation mode |
| `Tab` | Toggle side panels |
| `1`-`9` | Solo track N |

## First run

The first two minutes decide whether someone keeps the app, so this is
designed rather than left to chance:

1. Window opens with a demo piece already loaded and playing on Ember Classic
   — never an empty grey window with a "File → Open" hint
2. A single unobtrusive hint: *Drop a MIDI file anywhere to begin*
3. Drag-and-drop anywhere in the window loads
4. Everything works immediately at defaults; the panels are there when wanted

No tour, no modal, no account. The Embers quick-start pitch was "a video in
under two minutes," and matching that means the software must be useful
before it is configured.

## Accessibility

- Full keyboard navigation; nothing reachable only by mouse
- Respects the OS reduced-motion setting by damping camera shake and drift,
  which is the part of this app most likely to cause trouble
- UI scale follows the system DPI setting, with a manual override
- The **Minimal** and **Print** Designs exist partly for contrast needs
- Colorblind-safe default palettes; track colors are always also labeled by
  name, never encoded by color alone
