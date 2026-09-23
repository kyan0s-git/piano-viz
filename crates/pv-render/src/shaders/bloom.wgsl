// Progressive downsample/upsample bloom (Jimenez, "Next Generation Post
// Processing in Call of Duty: Advanced Warfare", 2014): wide, smooth, and
// free of flicker from small bright sources.

struct Pass {
    texel: vec2f,       // 1 / source size
    weight: f32,        // upsample: contribution of the smaller mip
    prefilter: f32,     // 1 on the first downsample
    threshold: f32,
    knee: f32,
    _pad: vec2f,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> pass_: Pass;

struct Out {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> Out {
    let uv = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
    return Out(vec4f(uv * 2.0 - 1.0, 0.0, 1.0), vec2f(uv.x, 1.0 - uv.y));
}

fn tap(uv: vec2f) -> vec3f {
    return textureSampleLevel(src, samp, uv, 0.0).rgb;
}

fn luma(c: vec3f) -> f32 { return dot(c, vec3f(0.2126, 0.7152, 0.0722)); }

/// Soft-knee threshold: keeps only what's bright enough to glow.
fn threshold(c: vec3f) -> vec3f {
    let br = max(c.r, max(c.g, c.b));
    let k = pass_.threshold * pass_.knee + 1e-5;
    var soft = clamp(br - pass_.threshold + k, 0.0, 2.0 * k);
    soft = soft * soft / (4.0 * k);
    return c * max(soft, br - pass_.threshold) / max(br, 1e-5);
}

@fragment
fn fs_down(in: Out) -> @location(0) vec4f {
    let t = pass_.texel;
    let a = tap(in.uv + t * vec2f(-2.0, 2.0));
    let b = tap(in.uv + t * vec2f(0.0, 2.0));
    let c = tap(in.uv + t * vec2f(2.0, 2.0));
    let d = tap(in.uv + t * vec2f(-2.0, 0.0));
    let e = tap(in.uv);
    let f = tap(in.uv + t * vec2f(2.0, 0.0));
    let g_ = tap(in.uv + t * vec2f(-2.0, -2.0));
    let h = tap(in.uv + t * vec2f(0.0, -2.0));
    let i = tap(in.uv + t * vec2f(2.0, -2.0));
    let j = tap(in.uv + t * vec2f(-1.0, 1.0));
    let k = tap(in.uv + t * vec2f(1.0, 1.0));
    let l = tap(in.uv + t * vec2f(-1.0, -1.0));
    let m = tap(in.uv + t * vec2f(1.0, -1.0));

    if pass_.prefilter > 0.5 {
        // Karis average on the first pass: weight each group by inverse
        // luma so one hot pixel can't produce a flickering blob.
        let g0 = (a + b + d + e) * 0.25;
        let g1 = (b + c + e + f) * 0.25;
        let g2 = (d + e + g_ + h) * 0.25;
        let g3 = (e + f + h + i) * 0.25;
        let g4 = (j + k + l + m) * 0.25;
        let w0 = 1.0 / (1.0 + luma(g0));
        let w1 = 1.0 / (1.0 + luma(g1));
        let w2 = 1.0 / (1.0 + luma(g2));
        let w3 = 1.0 / (1.0 + luma(g3));
        let w4 = 1.0 / (1.0 + luma(g4));
        let sum = g0 * w0 * 0.125 + g1 * w1 * 0.125 + g2 * w2 * 0.125 + g3 * w3 * 0.125 + g4 * w4 * 0.5;
        let norm = w0 * 0.125 + w1 * 0.125 + w2 * 0.125 + w3 * 0.125 + w4 * 0.5;
        return vec4f(threshold(sum / norm), 1.0);
    }
    var o = e * 0.125;
    o += (a + c + g_ + i) * 0.03125;
    o += (b + d + f + h) * 0.0625;
    o += (j + k + l + m) * 0.125;
    return vec4f(o, 1.0);
}

@fragment
fn fs_up(in: Out) -> @location(0) vec4f {
    let t = pass_.texel;
    var o = tap(in.uv) * 4.0;
    o += (tap(in.uv + vec2f(-t.x, 0.0)) + tap(in.uv + vec2f(t.x, 0.0))
        + tap(in.uv + vec2f(0.0, -t.y)) + tap(in.uv + vec2f(0.0, t.y))) * 2.0;
    o += tap(in.uv - t) + tap(in.uv + t) + tap(in.uv + vec2f(-t.x, t.y)) + tap(in.uv + vec2f(t.x, -t.y));
    return vec4f(o / 16.0 * pass_.weight, 1.0);
}
