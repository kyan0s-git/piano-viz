// Shared by every scene shader. Layouts mirror src/uniforms.rs exactly.

struct Globals {
    size: vec4f,         // w, h, 1/w, 1/h  (render target, supersampled)
    view: vec4f,         // time, px (pixels per 1080p pixel), frame, supersample
    geo: vec4f,       // keyboard bottom y, keyboard height, pixels per second, dir (+1 falling, -1 rising)
    camera: vec4f,       // offset x, offset y, zoom, hand split pitch
    keys: vec4u,         // lo, hi, color source, palette length
    note_a: vec4f,       // corner radius px, border px, opacity, intensity
    note_b: vec4f,       // gradient mode, falloff, margin px, active boost
    note_c: vec4f,       // velocity brightness, black-key shade, tail opacity, time of note being hit window
    border_color: vec4f,
    white_key: vec4f,
    black_key: vec4f,
    separator: vec4f,    // rgb, width px
    kb_a: vec4f,         // black width ratio, black length ratio, depress px, tint amount
    kb_b: vec4f,         // glow intensity, glow radius px, strike width px, strike intensity
    strike_color: vec4f,
    bg: vec4f,           // kind, stop count, angle (rad), radial
    bg_color: vec4f,     // solid color, or image tint
    bg_image: vec4f,     // fit, image aspect, has image, unused
    stops: array<vec4f, 8>,     // rgb + position
    palette: array<vec4f, 16>,
}

struct Note {
    start: f32,
    duration: f32,
    sustain: f32,
    packed: u32,         // pitch | velocity << 8 | track << 16 | flags << 24
}

struct Key {
    x0: f32,             // fraction of width
    x1: f32,
    black: f32,
    visible: f32,
    note: i32,           // index of the note holding it down, or -1
    press: f32,          // 0..1, decays after release
    age: f32,            // seconds since pressed
    velocity: f32,       // 0..1
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> notes: array<Note>;
@group(0) @binding(2) var<storage, read> tracks: array<vec4f>;   // rgb, visible
@group(0) @binding(3) var<storage, read> keys: array<Key>;

const FLAG_PEDAL: u32 = 1u;
const FLAG_BLACK: u32 = 2u;
const FLAG_PERCUSSION: u32 = 8u;

fn n_pitch(n: Note) -> u32 { return n.packed & 0xffu; }
fn n_velocity(n: Note) -> f32 { return f32((n.packed >> 8u) & 0xffu) / 127.0; }
fn n_track(n: Note) -> u32 { return (n.packed >> 16u) & 0xffu; }
fn n_flags(n: Note) -> u32 { return n.packed >> 24u; }
fn n_channel(n: Note) -> u32 { return n.packed >> 28u; }

fn now() -> f32 { return g.view.x; }
fn px() -> f32 { return g.view.y; }
fn kb_top() -> f32 { return g.geo.x + g.geo.y; }

/// Height on screen of a moment in the song.
fn time_to_y(time: f32) -> f32 {
    return kb_top() + g.geo.w * (time - now()) * g.geo.z;
}

fn palette(i: u32) -> vec3f {
    return g.palette[i % max(g.keys.w, 1u)].rgb;
}

/// Base color of a note, before intensity. Mirrors ColorSource in pv-design.
fn note_color(n: Note) -> vec3f {
    let p = n_pitch(n);
    let v = n_velocity(n);
    var c: vec3f;
    switch g.keys.z {
        case 0u: { c = tracks[n_track(n)].rgb; }
        case 1u: { c = palette(n_channel(n)); }
        case 2u: { c = palette(select(1u, 0u, f32(p) < g.camera.w)); }
        case 3u: { c = palette(p % 12u); }
        case 5u: {
            let len = max(g.keys.w, 1u);
            let f = v * f32(len - 1u);
            let i = min(u32(f), len - 1u);
            c = mix(palette(i), palette(min(i + 1u, len - 1u)), fract(f));
        }
        default: { c = palette(0u); }
    }
    if (n_flags(n) & FLAG_BLACK) != 0u {
        c *= g.note_c.y;
    }
    return c * mix(1.0, 0.4 + 0.8 * v, g.note_c.x);
}

fn key_center(p: u32) -> f32 {
    let k = keys[p];
    return (k.x0 + k.x1) * 0.5 * g.size.x;
}

/// Pixel position (y up) to clip space, through the camera.
fn to_clip(p: vec2f) -> vec4f {
    let c = g.size.xy * 0.5;
    let q = (p - c) * g.camera.z + c + g.camera.xy;
    return vec4f(q * g.size.zw * 2.0 - 1.0, 0.0, 1.0);
}

/// Clip position that the rasterizer throws away: a zero-area triangle.
const DEAD: vec4f = vec4f(-10.0, -10.0, 0.0, 1.0);

/// Corner of a unit quad for vertex 0..5 (two triangles).
fn quad_corner(vi: u32) -> vec2f {
    let x = array<f32, 6>(0.0, 1.0, 0.0, 1.0, 1.0, 0.0);
    let y = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    return vec2f(x[vi], y[vi]);
}

fn sd_round_box(p: vec2f, b: vec2f, r: f32) -> f32 {
    let q = abs(p) - b + r;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_box(p: vec2f, b: vec2f) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0);
}

// PCG hash: well distributed, cheap, and identical on every GPU, which is
// what keeps particle renders reproducible.
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed >> 8u) / 16777216.0;
}

/// Light added to the scene. Alpha records how much light, so a transparent
/// export keeps glow over an empty background instead of dropping it.
fn additive(c: vec3f) -> vec4f {
    return vec4f(c, min(max(c.r, max(c.g, c.b)), 1.0));
}
