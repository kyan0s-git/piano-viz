// Final pass: resolve supersampling, add bloom, reflect, tonemap, grade, and
// write the display image.

struct Post {
    bloom: vec4f,       // intensity (0 = off), unused, unused, unused
    bloom_tint: vec4f,
    tone: vec4f,        // operator, exposure (linear), saturation, alpha mode
    vignette: vec4f,    // amount, smoothness, grain amount, time (bits seed the grain)
    misc: vec4f,        // aberration, encode sRGB, supersample, dither
    reflect: vec4f,     // opacity (0 = off), fade, blur px, keyboard bottom (fraction of height)
    out_size: vec4f,    // w, h, 1/w, 1/h
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> post: Post;

struct Out { @builtin(position) pos: vec4f }

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> Out {
    let uv = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
    return Out(vec4f(uv * 2.0 - 1.0, 0.0, 1.0));
}

/// Exact box filter over the supersampled pixels behind one output pixel.
fn resolve(frag: vec2f) -> vec4f {
    let s = i32(post.misc.z);
    let base = vec2i(floor(frag)) * s;
    var acc = vec4f(0.0);
    for (var y = 0; y < s; y++) {
        for (var x = 0; x < s; x++) {
            acc += textureLoad(scene, base + vec2i(x, y), 0);
        }
    }
    return acc / f32(s * s);
}

fn scene_at(uv: vec2f) -> vec4f {
    return textureSampleLevel(scene, samp, uv, 0.0);
}

// AgX, via Benjamin Wrensch's polynomial fit. Desaturates gracefully as
// values blow out, where ACES clips saturated bright color to neon.
fn agx_contrast(x: vec3f) -> vec3f {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3f) -> vec3f {
    let inset = mat3x3f(
        0.842479062253094, 0.0423282422610123, 0.0423756549057051,
        0.0784335999999992, 0.878468636469772, 0.0784336,
        0.0792237451477643, 0.0791661274605434, 0.879142973793104);
    let outset = mat3x3f(
        1.19687900512017, -0.0528968517574562, -0.0529716355144438,
        -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
        -0.0990297440797205, -0.0989611768448433, 1.15107367264116);
    let lo = -12.47393;
    let hi = 4.026069;
    var v = inset * c;
    v = clamp(log2(max(v, vec3f(1e-10))), vec3f(lo), vec3f(hi));
    v = agx_contrast((v - lo) / (hi - lo));
    v = outset * v;
    // AgX's output is display-encoded; return linear for a uniform pipeline.
    return pow(clamp(v, vec3f(0.0), vec3f(1.0)), vec3f(2.2));
}

fn aces(x: vec3f) -> vec3f {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3f(0.0), vec3f(1.0));
}

fn tonemap(c: vec3f) -> vec3f {
    switch u32(post.tone.x) {
        case 0u: { return agx(c); }
        case 1u: { return aces(c); }
        case 2u: { return c / (1.0 + c); }
        default: { return clamp(c, vec3f(0.0), vec3f(1.0)); }
    }
}

fn to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

fn hash2(p: vec2f, seed: u32) -> f32 {
    let q = vec2u(vec2i(p)) ^ vec2u(seed * 1664525u);
    var h = q.x * 1597334677u ^ q.y * 3812015801u;
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return f32((h >> 22u) ^ h) / 4294967295.0;
}

@fragment
fn fs(in: Out) -> @location(0) vec4f {
    let frag = in.pos.xy;
    let uv = frag * post.out_size.zw;
    var c = resolve(frag);

    // Chromatic aberration: red and blue pulled apart radially.
    if post.misc.x > 0.0 {
        let dir = (uv - 0.5) * post.misc.x;
        c = vec4f(scene_at(uv + dir).r, c.g, scene_at(uv - dir).b, c.a);
    }

    // Reflection of the scene into the strip below the keyboard.
    let floor_y = 1.0 - post.reflect.w; // top-down uv of the keyboard's bottom edge
    if post.reflect.x > 0.0 && uv.y > floor_y {
        let depth = (uv.y - floor_y) / max(post.reflect.w, 1e-4);
        let m = vec2f(uv.x, floor_y - (uv.y - floor_y));
        let b = post.reflect.z * post.out_size.zw;
        let r = (scene_at(m) * 2.0 + scene_at(m + vec2f(b.x, 0.0)) + scene_at(m - vec2f(b.x, 0.0))
               + scene_at(m + vec2f(0.0, b.y)) + scene_at(m - vec2f(0.0, b.y))) / 6.0;
        let fade = post.reflect.x * (1.0 - smoothstep(0.0, max(post.reflect.y, 0.01), depth));
        c = vec4f(c.rgb + r.rgb * fade, c.a);
    }

    let bloom = textureSampleLevel(bloom_tex, samp, uv, 0.0).rgb * post.bloom.x * post.bloom_tint.rgb;
    var hdr = c.rgb + bloom;
    var alpha = 1.0;
    if post.tone.w > 0.5 {
        // Transparent export: glow over nothing becomes partial alpha, and
        // color is un-premultiplied so compositors see straight alpha.
        alpha = clamp(c.a + dot(bloom, vec3f(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
        hdr = hdr / max(alpha, 1e-4);
    }
    var col = tonemap(hdr * post.tone.y);

    let l = dot(col, vec3f(0.2126, 0.7152, 0.0722));
    col = max(mix(vec3f(l), col, post.tone.z), vec3f(0.0));

    if post.vignette.x > 0.0 {
        let d = length((uv - 0.5) * vec2f(post.out_size.x * post.out_size.w, 1.0)) / 0.9;
        col *= 1.0 - post.vignette.x * smoothstep(1.0 - post.vignette.y, 1.0 + post.vignette.y * 0.5, d);
    }

    if post.misc.y > 0.5 {
        col = to_srgb(clamp(col, vec3f(0.0), vec3f(1.0)));
    }
    // Grain, then triangular dither against 8-bit banding in dark gradients.
    // Seeded by the playhead rather than the frame number, so a moment looks
    // the same at any frame rate.
    let seed = bitcast<u32>(post.vignette.w);
    if post.vignette.z > 0.0 {
        col += (hash2(frag, seed) - 0.5) * post.vignette.z;
    }
    if post.misc.w > 0.0 {
        col += (hash2(frag + 17.0, seed) - hash2(frag + 71.0, seed ^ 0x5bd1e995u)) / 255.0;
    }
    return vec4f(clamp(col, vec3f(0.0), vec3f(1.0)), alpha);
}
