# 08 — Export

## The guarantee

What you preview is what you render. Not approximately — the same pipeline,
the same shaders, the same parameters, differing only in where time comes
from and where pixels go.

This is worth stating as a guarantee because the common alternative is a
separate "high quality render path" that drifts out of sync with the preview
over time, and users discover the difference only after a long export.

|  | Preview | Export |
|---|---|---|
| Time source | `AudioClock` (atomic from audio thread) | `FrameClock` (`n`/fps, rational) |
| Destination | swapchain surface | offscreen texture → readback |
| Resolution | window size | user-chosen |
| Supersampling | 1x default | 2x default |
| Everything else | — identical — | |

Because [particles are stateless](05-particles.md) and camera shake is
analytic, no part of the scene depends on frame history. Frame `n` is a pure
function of `n`. That's what makes the guarantee hold rather than merely
being intended.

## The frame pump

```
for n in 0..total_frames:
    t = n * fps_den / fps_num          ← exact rational, never accumulated
    render scene at t into HDR target
    post-process → LDR target
    copy to staging buffer (ring of 3)
    map a completed staging buffer (2 frames behind)
    write its bytes to the encoder's stdin
```

### Why a ring of three

Mapping a GPU buffer for CPU read stalls until the GPU finishes writing it.
Doing that on the frame just submitted means the CPU waits for the GPU and
then the GPU waits for the CPU, and neither is ever busy at the same time —
which typically halves throughput or worse.

With three staging buffers, the CPU maps the buffer from two frames ago,
which the GPU finished long since, while the GPU works on the current one.
Three rather than two because the copy, the map, and the encode are at
different points in the pipeline and two leaves no slack.

### Row padding

`wgpu` requires buffer copy rows aligned to 256 bytes. At widths that aren't
a multiple of 64 pixels, the staging buffer has padding at the end of every
row that must be stripped before handing bytes to the encoder. Getting this
wrong produces the classic diagonal-skew image. Mentioned here because it is
invisible until it isn't, and 1920 happens to be aligned while plenty of
custom resolutions are not.

## Encoding

We **pipe raw frames to an `ffmpeg` subprocess** rather than linking a codec
library.

The reason is licensing. Linking FFmpeg pulls in LGPL at minimum and GPL with
x264 — which would force this project's license and prevent distributing a
static binary. Invoking a separate executable the user already has is an
arm's-length use that carries no such obligation, and it's how most
permissively-licensed tools handle this.

It also means every codec and container FFmpeg supports works with no
per-format code, and users can pass their own flags for cases we didn't
anticipate.

```
ffmpeg -f rawvideo -pix_fmt rgba -s 1920x1080 -r 60 -i -   \
       -i audio.wav -c:v libx264 -crf 18 -preset medium    \
       -pix_fmt yuv420p -c:a aac -b:a 320k out.mp4
```

### Finding ffmpeg

1. A path set in preferences
2. Alongside our binary (for anyone who wants to bundle it themselves)
3. On `PATH`
4. If absent: say so clearly, link to install instructions per platform, and
   **offer PNG-sequence export as a working fallback** — which needs no
   external tool and is what compositors often want anyway.

Never a silent failure at the end of a long render. The check runs *before*
the first frame, not after the last.

## Output presets

| Preset | Codec | Notes |
|---|---|---|
| YouTube 1080p60 | H.264, CRF 18, yuv420p | the default |
| YouTube 4K60 | H.264, CRF 17 | |
| High quality | H.265, CRF 20 | smaller at equal quality |
| **Alpha (ProRes 4444)** | ProRes 4444 | for Premiere / After Effects / Resolve |
| **Alpha (WebM)** | VP9 + alpha | for web compositing |
| PNG sequence | — | no ffmpeg needed; lossless |
| Lossless | FFV1 | archival |

Alpha export is the feature Embers never had, and it matters more than its
obscurity suggests: keying a glow effect against a solid background destroys
exactly the soft bright edges that make the look work. Real alpha keeps them.
With alpha enabled the background layer is skipped and the HDR target is
tonemapped with premultiplied alpha preserved.

## Range and options

- **Range** — whole piece, a time range, or the loop region
- **Resolution** — presets plus arbitrary, with an aspect lock
- **Framerate** — 24 / 25 / 30 / 50 / 60 / 120, plus 29.97 and 59.94 as exact
  rationals
- **Supersampling** — 1x / 1.5x / 2x / 4x
- **Tail** — extra seconds after the last note so particles and bloom finish
  decaying rather than cutting off mid-fade. Defaults to the longest particle
  lifetime plus one second, which is the right answer often enough that most
  people will never touch it.

## Progress and control

- Frames done / total, elapsed, ETA from a rolling average of recent frames
- A live thumbnail of the frame being rendered, updated a few times a second
  — cheap, and it's the difference between trusting a long render and
  babysitting it
- Cancel, which cleans up the partial file rather than leaving a corrupt one
- Pause and resume
- On failure: the encoder's stderr, verbatim, not a generic message

## The CLI

`pv-cli` is the same engine with no window:

```
pv render song.mid --design ember.pvd -o out.mp4 \
   --resolution 1920x1080 --fps 60 --ss 2

pv render song.mid --design ember.pvd -o out.mov --alpha
pv batch ./midis/ --design ember.pvd --out ./renders/
pv inspect song.mid          # note count, duration, tracks, tempo map
```

This is the piece Embers never had. It makes batch rendering possible, makes
the app usable from scripts and CI, and — not incidentally — is what lets the
[golden-image tests](12-testing.md) run headless.

Requires a GPU adapter but no display server, so it works over SSH and in
containers with a software adapter.
