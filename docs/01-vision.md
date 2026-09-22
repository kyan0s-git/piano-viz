# 01 — Vision & scope

## The goal

Someone drops a piano MIDI on the window and within seconds has something
that looks like an Embers video. They tune it to taste, hit render, and get
an MP4 they can upload.

Two audiences, and the design serves both:

- **The casual user** wants a good result from defaults. They should never
  have to open a settings panel to get something worth posting.
- **The power user** wants control over every parameter, reproducible
  renders, and batch processing. They should never hit a ceiling.

## What Embers got right

Worth being explicit, because these are the things we must not lose:

- **Real-time preview with full effects.** No render-to-see-it loop. The
  feedback cycle is what makes visual tuning possible at all.
- **The look.** Bright, blown-out notes with heavy bloom, particle bursts on
  impact, a keyboard that lights up. Tasteful defaults, not a parameter soup.
- **"Designs"** — a separable bundle of colors and effects, distinct from
  playback settings. Users share Designs; that's the ecosystem.
- **Fast renders.** Exporting is not an overnight job.

## What Embers got wrong, or never got to

These are our openings, not criticisms of a project that did a lot right:

- **Windows-only.** Cut off macOS and Linux users entirely.
- **Closed source.** When development stopped, everything stopped. No one
  could pick it up. This is the failure mode we're structurally avoiding.
- **Watermark unless you pay** for rendering. Ours has no watermark, ever.
- **No alpha export.** Creators who composite in Premiere/Resolve/AE have to
  chroma-key, which destroys glow edges — exactly the part that looks good.
- **No headless CLI.** No batch rendering, no automation, no CI.
- **Binary settings.** Designs can't be diffed, version-controlled, or
  hand-edited. Ours are plain text.

## Scope

### In scope for v1 (full parity, per the brief)

- MIDI file loading, playback, transport, scrubbing, looping
- Falling-note visualization with a configurable piano keyboard
- Particle effects, bloom/glow, post-processing, camera motion
- Full Design system: colors, gradients, per-track/per-hand/per-pitch schemes
- SoundFont (SF2) synthesis with a bundled piano
- Real-time preview at 1080p60
- Video export: H.264/H.265/VP9/ProRes, with alpha, any resolution/fps
- **Live play mode** — a connected MIDI keyboard drives the visuals
- Headless CLI for batch rendering

### Explicitly not in scope

- **A DAW.** No recording, mixing, or arrangement.
- **A MIDI editor.** We read MIDI. Light touch-ups (track mute/solo, tempo
  override, time shift) yes; a piano-roll editor no.
- **Non-piano instruments.** The brief says piano only. The architecture
  doesn't *preclude* a future guitar/drum layout — the keyboard is one
  implementation of an `InstrumentLayout` — but nothing else is built for it.
- **Sheet music / notation rendering.** Different problem, different project.
- **Audio effects.** No reverb bus, EQ, or mastering. The synth's built-in
  reverb is the extent of it.
- **A plugin marketplace.** Designs are shared as files. That's enough.

### Deliberately deferred

Things we want, sequenced after v1 rather than cut:

- Video backgrounds (decoding video on the preview thread is its own project)
- Custom user shaders for backgrounds and effects
- Multi-instrument / ensemble layouts
- Cloud or distributed rendering

## Success criteria

Concrete enough to test against:

1. A first-time user gets a video they'd post, from defaults, in under two
   minutes — the bar Embers set with its own quick-start.
2. 1080p60 preview holds 60fps on Intel Iris Xe-class integrated graphics
   with a dense MIDI loaded.
3. Export of a 3-minute piece at 1080p60 finishes in under 90 seconds on a
   mid-range discrete GPU.
4. A render is bit-identical across runs on the same machine.
5. Preview at time *t* and the exported frame at time *t* match within a
   negligible perceptual threshold.
6. A dense "black MIDI" (500k+ notes) loads and plays without falling over.
