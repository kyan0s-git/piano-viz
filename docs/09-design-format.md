# 09 — Design format

A **Design** is the look: colors, effects, and layout. It is deliberately
separate from the session (playhead, mutes, output path) so that sharing a
Design shares only the look — nobody wants someone else's file paths.

The name is borrowed from Embers, which used the same split between OPTIONS
and Designs. It was the right split.

## Format: TOML

Considered and rejected:

- **JSON** — no comments. For a file people hand-edit and share, that alone
  disqualifies it.
- **RON** — clean enum support and Rust-native, but unfamiliar to anyone
  outside Rust, and Designs should be editable by artists, not just
  programmers.
- **Binary** — fastest, and what Embers used. But it can't be diffed,
  reviewed, version-controlled, or edited in a text editor. For a format
  whose purpose is community sharing, those losses outweigh parse speed on a
  file measured in kilobytes.

TOML: comments, human-readable, unambiguous, and `serde` handles tagged enums
for effect variants with `#[serde(tag = "type")]`.

## Bundle layout

A Design is a directory, or a zip of one with the extension `.pvd`:

```
ember-classic.pvd/
  design.toml        the parameters
  preview.png        thumbnail for the browser
  assets/
    background.jpg
    font.ttf
```

A directory during authoring (hot-reload works on it), zipped for
distribution. Assets are referenced by relative path and may not escape the
bundle — a `../` in an asset path is rejected at load, since Designs are
files people download from strangers.

## Schema

```toml
[meta]
name = "Ember Classic"
author = "kyan0s"
version = 1                    # schema version, not design version
description = "Warm sparks on a dark field"

# ── Background ───────────────────────────────────────────────
[background]
type = "gradient"              # solid | gradient | image
stops = [
  { at = 0.0, color = "#05060a" },
  { at = 1.0, color = "#120a06" },
]
angle = 90.0

[background.reflection]
enabled = true
opacity = 0.25
blur = 4.0
fade = 0.7

# ── Notes ────────────────────────────────────────────────────
[notes]
corner_radius = 4.0
border_width = 1.5
border_color = "#ffffff60"
opacity = 1.0
intensity = 2.2                # HDR multiplier — drives bloom strength
gradient = "along"             # none | along | across
gradient_falloff = 0.35

[notes.color]
source = "track"               # track | channel | hand | pitch_class | fixed | velocity
palette = ["#ff6b35", "#f7c59f", "#efefd0", "#4f9d9d"]
velocity_brightness = 0.4      # how much velocity scales brightness

# ── Keyboard ─────────────────────────────────────────────────
[keyboard]
range = "auto"                 # auto | "88" | "76" | "61" | "21-108"
height = 0.16                  # fraction of viewport height
white_key_color = "#e8e8e8"
black_key_color = "#0d0d0d"
black_width_ratio = 0.58
black_length_ratio = 0.62
separator_width = 1.0

[keyboard.pressed]
tint_from_note = true           # pressed key takes the note's color
glow_intensity = 1.8
glow_radius = 24.0
depress_pixels = 2.0            # keys visibly move

# ── Particles ────────────────────────────────────────────────
[[particles]]
event = "note_hit"
count = 48
lifetime = 1.4
lifetime_variance = 0.35
speed = 260.0
speed_variance = 0.4
spread_degrees = 70.0
gravity = [0.0, -180.0]
drag = 0.9
size = 3.5
size_curve = "shrink"           # constant | shrink | grow | pulse
opacity_curve = "fade_out"
turbulence = 30.0
color_source = "note"           # note | palette | fixed
intensity = 3.0
velocity_response = 0.7         # MIDI velocity scales count/speed/size

[[particles]]
event = "note_hold"
rate = 14.0                     # per second while held
lifetime = 2.0
speed = 40.0
spread_degrees = 25.0
gravity = [0.0, 60.0]           # embers rise
size = 2.0
intensity = 2.0

# ── Post-processing ──────────────────────────────────────────
[post.bloom]
enabled = true
threshold = 0.85
soft_knee = 0.5
intensity = 0.9
radius = 1.0
tint = "#ffd9b0"

[post.tonemap]
operator = "agx"               # agx | aces | reinhard | none
exposure = 1.0

[post.vignette]
enabled = true
amount = 0.35
smoothness = 0.5

[post.grain]
enabled = true
amount = 0.03

[post.chromatic_aberration]
enabled = false
amount = 0.002

# ── Camera ───────────────────────────────────────────────────
[camera]
zoom = 1.0
[camera.shake]
enabled = true
amount = 2.5
decay = 8.0
velocity_response = 1.0
[camera.drift]
enabled = false
amount = 6.0
speed = 0.08

# ── Layout ───────────────────────────────────────────────────
[layout]
lookahead = 3.0                # seconds of future visible
direction = "down"             # down | up
strike_line = 0.16             # matches keyboard height
```

## Color notation

Hex strings, with alpha optional: `"#ff6b35"` or `"#ff6b35c0"`. Parsed as
sRGB and converted to linear on load, once — not per frame, and never mixed.

Values above 1.0 for HDR are expressed through the separate `intensity`
multipliers rather than by allowing out-of-range hex, which would be
unintuitive and unrepresentable in a color picker.

## Versioning

`meta.version` is the schema version. Loading rules:

- **Older schema** → migrate forward through a chain of migration functions,
  each one version step. Keeps migrations small and individually testable.
- **Newer schema** → load what's recognized, warn about the rest, don't fail.
  A Design from a newer build should still mostly work.
- **Unknown key** → warn, ignore. Never an error.

Being lenient here is deliberate. These files circulate between users on
different versions, and a hard failure on an unrecognized key would make
sharing fragile for no real safety benefit.

## Hot reload

The Design file is watched (`notify`). On change: reparse, validate, swap.
Playback doesn't stop and the playhead doesn't move.

This turns an external text editor into a live tuning surface, which is
genuinely faster than any GUI for anyone comfortable with it — edit, save,
watch it change mid-playback. Parse errors surface as a non-blocking banner
with the line number, keeping the last good Design active rather than
reverting to defaults and losing the work in progress.

## Built-in Designs

Ship several, since defaults are most of what most users ever see:

- **Ember Classic** — the reference look: warm sparks, heavy bloom, dark
- **Aurora** — cool gradients, soft particles, slow drift
- **Minimal** — flat colors, no particles, light background; for tutorials
  where the notes need to be legible rather than pretty
- **Neon** — high-saturation, strong aberration, black background
- **Print** — white background, dark notes, no effects; for screenshots and
  documentation

They double as schema documentation: every parameter appears in at least one
shipped Design, so there's always a working example to copy from.
