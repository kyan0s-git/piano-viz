// Falling notes: one instance per note, drawn twice — white-key notes, then
// black-key notes on top — so overlap always resolves the way a keyboard does.

override BLACK_PASS: bool = false;

struct NoteOut {
    @builtin(position) pos: vec4f,
    @location(0) world: vec2f,
    @location(1) @interpolate(flat) body: vec4f,   // x0, y0, x1, y1
    @location(2) @interpolate(flat) tail: vec2f,   // y0, y1
    @location(3) @interpolate(flat) color: vec4f,  // rgb (HDR), active 0/1
    @location(4) @interpolate(flat) span: vec2f,   // start, duration
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> NoteOut {
    var out: NoteOut;
    let n = notes[ii];
    let p = n_pitch(n);
    let k = keys[p];
    let hidden = tracks[n_track(n)].a < 0.5
        || (n_flags(n) & FLAG_PERCUSSION) != 0u
        || k.visible < 0.5
        || ((n_flags(n) & FLAG_BLACK) != 0u) != BLACK_PASS;
    if hidden {
        out.pos = DEAD;
        return out;
    }

    let margin = g.note_b.z;
    let x0 = k.x0 * g.size.x + margin;
    let x1 = max(k.x1 * g.size.x - margin, x0 + 1.0);
    let ya = time_to_y(n.start);
    let yb = time_to_y(n.start + n.duration);
    let body = vec4f(x0, min(ya, yb), x1, max(ya, yb));

    var tail = vec2f(0.0);
    if g.note_c.z > 0.0 && n.sustain > 0.0 {
        let yc = time_to_y(n.start + n.duration + n.sustain);
        tail = vec2f(min(yb, yc), max(yb, yc));
    }

    // Quad covering body and tail, plus a pixel for antialiasing.
    let lo = vec2f(x0 - 1.0, min(body.y, select(body.y, tail.x, tail.y > tail.x)) - 1.0);
    let hi = vec2f(x1 + 1.0, max(body.w, select(body.w, tail.y, tail.y > tail.x)) + 1.0);
    // Entirely below the keys or above the frame: skip.
    if hi.y < kb_top() || lo.y > g.size.y + 4.0 {
        out.pos = DEAD;
        return out;
    }
    let world = mix(lo, hi, quad_corner(vi));

    let is_active = select(0.0, 1.0, now() >= n.start && now() < n.start + n.duration);
    out.pos = to_clip(world);
    out.world = world;
    out.body = body;
    out.tail = tail;
    out.color = vec4f(note_color(n) * g.note_a.w, is_active);
    out.span = vec2f(n.start, n.duration);
    return out;
}

@fragment
fn fs(in: NoteOut) -> @location(0) vec4f {
    let p = in.world;
    if p.y < kb_top() { discard; }

    let c = (in.body.xy + in.body.zw) * 0.5;
    let half = (in.body.zw - in.body.xy) * 0.5;
    let r = min(g.note_a.x, min(half.x, half.y));
    let d = sd_round_box(p - c, half, r);
    let body_a = clamp(0.5 - d, 0.0, 1.0);

    // Shading along or across the note.
    var shade = 1.0;
    let mode = u32(g.note_b.x);
    if mode == 1u {
        // Brightest at the edge that reaches the keys first.
        let time_here = now() + (p.y - kb_top()) / (g.geo.w * g.geo.z);
        let s = clamp((time_here - in.span.x) / max(in.span.y, 1e-4), 0.0, 1.0);
        shade = 1.0 - g.note_b.y * s;
    } else if mode == 2u {
        let u = (p.x - c.x) / max(half.x, 1e-3);
        shade = 1.0 - g.note_b.y * u * u;
    }
    let boost = mix(1.0, g.note_b.w, in.color.a);
    let fill = in.color.rgb * shade * boost;

    // Border: the band within border px of the edge.
    let inner_a = clamp(0.5 - (d + g.note_a.y), 0.0, 1.0);
    let bc = g.border_color;
    let ring = max(body_a - inner_a, 0.0) * bc.a;
    var out = vec4f(fill * inner_a + bc.rgb * g.note_a.w * boost * ring, inner_a + ring);

    // Pedal tail, underneath the body.
    if in.tail.y > in.tail.x {
        let tc = vec2f(c.x, (in.tail.x + in.tail.y) * 0.5);
        let th = vec2f(half.x * 0.6, (in.tail.y - in.tail.x) * 0.5);
        let ta = clamp(0.5 - sd_box(p - tc, th), 0.0, 1.0) * g.note_c.z;
        // Fade toward the far end, like a decaying string.
        let time_here = now() + (p.y - kb_top()) / (g.geo.w * g.geo.z);
        let decay = 1.0 - clamp((time_here - (in.span.x + in.span.y)) / max(in.tail.y - in.tail.x, 1.0) * g.geo.z, 0.0, 1.0) * 0.7;
        let tail = vec4f(in.color.rgb * 0.6 * decay, 1.0) * ta;
        out = out + tail * (1.0 - out.a);
    }
    return out * g.note_a.z;
}
