// Keys, drawn as two instanced passes (white, then black on top), then the
// additive glow above pressed keys and the strike line.

override BLACK_PASS: bool = false;

struct KeyOut {
    @builtin(position) pos: vec4f,
    @location(0) world: vec2f,
    @location(1) @interpolate(flat) rect: vec4f,   // x0, y0, x1, y1 of the key
    @location(2) @interpolate(flat) pitch: u32,
}

fn shadow_px() -> f32 { return 4.0 * px(); }

@vertex
fn vs_key(@builtin(vertex_index) vi: u32, @builtin(instance_index) p: u32) -> KeyOut {
    var out: KeyOut;
    let k = keys[p];
    if k.visible < 0.5 || (k.black > 0.5) != BLACK_PASS {
        out.pos = DEAD;
        return out;
    }
    let x0 = k.x0 * g.size.x;
    let x1 = k.x1 * g.size.x;
    let top = kb_top();
    var y0 = g.geo.x;
    var grow = vec2f(0.0);
    if BLACK_PASS {
        y0 = top - g.geo.y * g.kb_a.y;
        grow = vec2f(shadow_px(), shadow_px());
    }
    let lo = vec2f(x0 - grow.x, y0 - grow.y);
    let hi = vec2f(x1 + grow.x, top);
    let world = mix(lo, hi, quad_corner(vi));
    out.pos = to_clip(world);
    out.world = world;
    out.rect = vec4f(x0, y0, x1, top);
    out.pitch = p;
    return out;
}

/// Color of the note holding a key. Glow intensity brightens it for bloom
/// but never darkens it: a Design with no glow still tints pressed keys.
fn press_color(k: Key) -> vec3f {
    if k.note < 0 { return vec3f(0.0); }
    return note_color(notes[u32(k.note)]) * max(g.kb_b.x, 1.0);
}

@fragment
fn fs_key(in: KeyOut) -> @location(0) vec4f {
    let k = keys[in.pitch];
    let p = in.world;
    let w = in.rect.z - in.rect.x;
    let h = in.rect.w - in.rect.y;
    let press = k.press;
    // Depressed keys sink: shift the shading down.
    let sink = g.kb_a.z * press;
    let u = (p.x - in.rect.x) / w;
    let v = clamp((in.rect.w - sink - p.y) / h, 0.0, 1.0); // 0 at the top, 1 at the front

    if !BLACK_PASS {
        var c = g.white_key.rgb;
        c *= 0.84 + 0.16 * smoothstep(0.0, 0.85, v);           // lit from the front
        c *= mix(0.55, 1.0, smoothstep(0.0, 0.035, v));         // shadow under the fallboard
        c *= select(1.0, 0.78, v > 0.965);                      // front lip
        c = mix(c, press_color(k) * (0.75 + 0.25 * v), g.kb_a.w * press);
        // Separator between neighbors.
        let edge = min(p.x - in.rect.x, in.rect.z - p.x);
        let sep = smoothstep(g.separator.w * 0.5, g.separator.w * 0.5 + 1.0, edge);
        c = mix(g.separator.rgb, c, sep);
        return vec4f(c, 1.0);
    }

    // Black keys cast a soft shadow onto the white keys around them.
    let center = (in.rect.xy + in.rect.zw) * 0.5;
    let d = sd_box(p - center, (in.rect.zw - in.rect.xy) * 0.5);
    if d > 0.0 {
        let a = (1.0 - smoothstep(0.0, shadow_px(), d)) * 0.5;
        return vec4f(0.0, 0.0, 0.0, a);
    }
    var c = g.black_key.rgb;
    let front = smoothstep(0.84, 0.9, v);                        // the sloped front face
    let side = 1.0 - smoothstep(0.0, 0.14, min(u, 1.0 - u));     // bevelled sides
    c = c + vec3f(0.05) * front + vec3f(0.025) * side * (1.0 - front);
    c = c * (0.85 + 0.15 * (1.0 - v));
    c = mix(c, press_color(k) * (0.6 + 0.4 * front), g.kb_a.w * press);
    return vec4f(c, 1.0);
}

// ── Glow above pressed keys (additive) ──────────────────────────────────────

struct GlowOut {
    @builtin(position) pos: vec4f,
    @location(0) world: vec2f,
    @location(1) @interpolate(flat) info: vec4f,   // center x, half width, radius, strength
    @location(2) @interpolate(flat) color: vec3f,
}

@vertex
fn vs_glow(@builtin(vertex_index) vi: u32, @builtin(instance_index) p: u32) -> GlowOut {
    var out: GlowOut;
    let k = keys[p];
    let r = g.kb_b.y;
    if k.visible < 0.5 || k.press <= 0.001 || k.note < 0 || r <= 0.0 || g.kb_b.x <= 0.0 {
        out.pos = DEAD;
        return out;
    }
    let cx = key_center(p);
    let hw = (k.x1 - k.x0) * g.size.x * 0.5;
    let top = kb_top();
    let lo = vec2f(cx - hw - r * 1.5, top - r * 0.5);
    let hi = vec2f(cx + hw + r * 1.5, top + r * 4.0);
    let world = mix(lo, hi, quad_corner(vi));
    // A bright flash at the moment of the strike, settling to a steady glow.
    let flash = 1.0 + 1.5 * exp(-k.age * 10.0);
    out.pos = to_clip(world);
    out.world = world;
    out.info = vec4f(cx, hw, r, k.press * flash * (0.5 + 0.5 * k.velocity));
    out.color = note_color(notes[u32(k.note)]) * g.kb_b.x;
    return out;
}

@fragment
fn fs_glow(in: GlowOut) -> @location(0) vec4f {
    let dx = max(abs(in.world.x - in.info.x) - in.info.y * 0.6, 0.0) / (in.info.z * 0.6);
    let dy = (in.world.y - kb_top()) / in.info.z;
    let fall = exp(-dx * dx) * select(exp(-dy * 1.1), exp(-dy * dy * 16.0), dy < 0.0);
    return additive(in.color * fall * in.info.w * 0.35);
}

// ── Strike line (additive) ──────────────────────────────────────────────────

struct LineOut {
    @builtin(position) pos: vec4f,
    @location(0) world: vec2f,
}

@vertex
fn vs_line(@builtin(vertex_index) vi: u32) -> LineOut {
    let w = max(g.kb_b.z, 0.5) * 4.0;
    let lo = vec2f(0.0, kb_top() - w);
    let hi = vec2f(g.size.x, kb_top() + w);
    let world = mix(lo, hi, quad_corner(vi));
    return LineOut(to_clip(world), world);
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4f {
    let d = (in.world.y - kb_top()) / max(g.kb_b.z, 0.5);
    let core = exp(-d * d * 2.0);
    let halo = exp(-abs(d) * 0.9) * 0.25;
    return additive(g.strike_color.rgb * g.kb_b.w * (core + halo));
}
