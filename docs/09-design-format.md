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

A Design is a single `.toml` file, or a directory holding `design.toml` plus
its assets. (Zipped `.pvd` bundles for distribution are planned; v1 reads
directories, which keeps a zip library out of the binary until it's needed.)

```
ember-classic.pvd/
  design.toml        the parameters
  preview.png        thumbnail for the browser
  assets/
    background.jpg
    font.ttf
```

Assets are referenced by relative path and may not escape the bundle — a `../` in an asset path is rejected at load, since Designs are
files people download from strangers.

## Schema

The authoritative reference is
[`crates/pv-design/designs/ember-classic.toml`](../crates/pv-design/designs/ember-classic.toml),
which spells out every setting with comments. A test asserts it equals the
code's defaults, so the reference can't drift from the implementation.

Other Designs list only what they change; everything else falls back to the
defaults. A complete, working Design can be this short:

```toml
[meta]
name = "Night Blue"

[background]
type = "solid"
color = "#040814"

[notes.color]
source = "hand"
palette = ["#3fd6c8", "#8a7dff"]

[post.bloom]
intensity = 1.2
```

Top-level sections: `meta`, `background` (with `reflection`), `notes` (with
`color` and `sustain_tail`), `keyboard` (with `strike_line` and `pressed`),
`particles` (an array of emitters), `post` (`bloom`, `tonemap`, `vignette`,
`grain`, `chromatic_aberration`), `camera` (`shake`, `drift`), and `layout`.

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
