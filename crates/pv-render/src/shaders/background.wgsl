@group(0) @binding(6) var bg_tex: texture_2d<f32>;
@group(0) @binding(7) var bg_samp: sampler;

struct BgOut { @builtin(position) pos: vec4f }

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> BgOut {
    // One triangle covering the screen.
    let uv = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
    return BgOut(vec4f(uv * 2.0 - 1.0, 0.0, 1.0));
}

fn gradient(t: f32) -> vec3f {
    let n = u32(g.bg.y);
    if n == 0u { return g.bg_color.rgb; }
    var c = g.stops[0].rgb;
    for (var i = 1u; i < n; i++) {
        let a = g.stops[i - 1u];
        let b = g.stops[i];
        let f = clamp((t - a.w) / max(b.w - a.w, 1e-5), 0.0, 1.0);
        c = select(c, mix(a.rgb, b.rgb, f), t >= a.w);
    }
    return c;
}

@fragment
fn fs(in: BgOut) -> @location(0) vec4f {
    // y up, matching the scene.
    let uv = vec2f(in.pos.x * g.size.z, 1.0 - in.pos.y * g.size.w);
    let kind = u32(g.bg.x);
    var c = g.bg_color.rgb;
    if kind == 1u {
        var t: f32;
        if g.bg.w > 0.5 {
            let aspect = g.size.x * g.size.w;
            t = length((uv - 0.5) * vec2f(aspect, 1.0)) / length(vec2f(aspect, 1.0) * 0.5);
        } else {
            let d = vec2f(cos(g.bg.z), sin(g.bg.z));
            t = dot(uv - 0.5, d) / (abs(d.x) + abs(d.y)) + 0.5;
        }
        c = gradient(clamp(t, 0.0, 1.0));
    } else if kind == 2u && g.bg_image.z > 0.5 {
        // Fit modes: 0 fill (cover), 1 fit (contain), 2 stretch.
        let frame_aspect = g.size.x * g.size.w;
        let r = frame_aspect / g.bg_image.y;
        var s = vec2f(1.0);
        let fit = u32(g.bg_image.x);
        if fit == 0u { s = select(vec2f(1.0, 1.0 / r), vec2f(r, 1.0), r < 1.0); }
        if fit == 1u { s = select(vec2f(r, 1.0), vec2f(1.0, 1.0 / r), r < 1.0); }
        let iuv = (vec2f(uv.x, 1.0 - uv.y) - 0.5) * s + 0.5;
        let inside = all(iuv >= vec2f(0.0)) && all(iuv <= vec2f(1.0));
        let texel = textureSampleLevel(bg_tex, bg_samp, iuv, 0.0).rgb;
        c = select(vec3f(0.0), texel * g.bg_color.rgb, inside);
    }
    return vec4f(c, 1.0);
}
