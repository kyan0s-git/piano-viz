// Stateless particles (docs/05-particles.md). A particle's position is a
// closed-form function of its seed and age; nothing is stored or stepped
// between frames, so seeking is free and renders are reproducible.

struct Emitter {
    a: vec4f,   // event, count, rate, lifetime
    b: vec4f,   // lifetime variance, speed, speed variance, direction (rad)
    c: vec4f,   // spread (rad), gravity x, gravity y, drag
    d: vec4f,   // size, size variance, size curve, opacity curve
    e: vec4f,   // turbulence, color source, white-hot, intensity
    f: vec4f,   // fixed color rgb, velocity response
}

struct Particles {
    em: array<Emitter, 4>,
    info: vec4u,    // emitter count, instances per note
    offs: vec4u,    // first instance of each emitter within a note's block
    counts: vec4u,  // instances per note for each emitter
}

@group(0) @binding(4) var<storage, read> live: array<u32>;
@group(0) @binding(5) var<uniform> pe: Particles;

const EV_HIT: u32 = 0u;
const EV_HOLD: u32 = 1u;
const EV_RELEASE: u32 = 2u;

struct PartOut {
    @builtin(position) pos: vec4f,
    @location(0) local: vec2f,                     // -1..1 across the sprite
    @location(1) @interpolate(flat) color: vec3f,
}

fn curve(kind: u32, s: f32) -> f32 {
    switch kind {
        case 1u: { return 1.0 - s; }
        case 2u: { return s; }
        case 3u: { return select(1.0 - (s - 0.15) / 0.85, s / 0.15, s < 0.15); }
        case 4u: { return 1.0 - smoothstep(0.5, 1.0, s); }
        default: { return 1.0; }
    }
}

/// Position under gravity and linear drag after `t` seconds: the integral of
/// dv/dt = g - k v, with no timestep.
fn ballistic(v0: vec2f, grav: vec2f, k: f32, t: f32) -> vec2f {
    if k < 1e-4 {
        return v0 * t + 0.5 * grav * t * t;
    }
    let e = 1.0 - exp(-k * t);
    return (v0 + grav / k) * (e / k) - grav * t / k;
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PartOut {
    var out: PartOut;
    out.pos = DEAD;

    let stride = max(pe.info.y, 1u);
    let slot = ii / stride;
    let k = ii % stride;
    var e = 0u;
    for (var i = 1u; i < pe.info.x; i++) {
        if k >= pe.offs[i] { e = i; }
    }
    let j = k - pe.offs[e];
    let em = pe.em[e];
    let ni = live[slot];
    let n = notes[ni];

    let t = now();
    let event = u32(em.a.x);
    let life0 = em.a.w;
    let vel = n_velocity(n);
    let vk = mix(1.0, 0.35 + 1.1 * vel, em.f.w);

    // Which particle of this note's stream is this, and when was it born?
    var idx = j;
    var spawn = n.start;
    if event == EV_HOLD {
        let rate = max(em.a.z, 1e-3);
        let first = u32(max(ceil((t - life0 - n.start) * rate), 0.0));
        idx = first + j;
        spawn = n.start + f32(idx) / rate;
        if spawn > min(t, n.start + n.duration) { return out; }
    } else {
        if f32(j) >= em.a.y * vk { return out; }
        if event == EV_RELEASE { spawn = n.start + n.duration; }
    }

    var seed = pcg(ni * 0x9E3779B1u ^ pcg(idx * 4u + e));
    let life = life0 * (1.0 - em.b.x * rand(&seed));
    let age = t - spawn;
    if age < 0.0 || age > life { return out; }
    let s = age / life;

    let p = n_pitch(n);
    let kw = (keys[p].x1 - keys[p].x0) * g.size.x;
    let origin = vec2f(key_center(p) + (rand(&seed) - 0.5) * kw * 0.8, kb_top());
    let ang = em.b.w + (rand(&seed) - 0.5) * em.c.x;
    let speed = em.b.y * (1.0 - em.b.z * rand(&seed)) * vk * px();
    var pos = origin + ballistic(vec2f(cos(ang), sin(ang)) * speed, em.c.yz * px(), em.c.w, age);

    // Drift: bounded, closed-form, and growing with age so a burst starts
    // coherent and wanders apart.
    let ph = vec2f(rand(&seed), rand(&seed)) * 6.2831853;
    let fr = 1.0 + rand(&seed) * 2.0;
    pos += vec2f(sin(ph.x + age * fr), cos(ph.y + age * fr * 0.7)) * em.e.x * px() * age;

    let size = em.d.x * px() * (1.0 - em.d.y * rand(&seed)) * curve(u32(em.d.z), s) * sqrt(vk);
    let alpha = curve(u32(em.d.w), s);
    if size <= 0.01 || alpha <= 0.001 { return out; }

    var c: vec3f;
    switch u32(em.e.y) {
        case 1u: { c = palette(pcg(seed)); }
        case 2u: { c = em.f.rgb; }
        default: { c = note_color(n); }
    }
    let hot = em.e.z * (1.0 - s) * (1.0 - s);
    c = mix(c, vec3f(1.0, 0.97, 0.92), hot) * em.e.w * alpha;

    // The sprite reaches past the core radius for its soft halo.
    let corner = quad_corner(vi) * 2.0 - 1.0;
    let r = size * 3.0 + 0.75;
    out.pos = to_clip(pos + corner * r);
    out.local = corner * (r / max(size, 0.25));
    out.color = c;
    return out;
}

@fragment
fn fs(in: PartOut) -> @location(0) vec4f {
    let d2 = dot(in.local, in.local);
    let core = exp(-d2 * 1.5);
    let halo = exp(-d2 * 0.25) * 0.12;
    return additive(in.color * (core + halo));
}
