// Visuales de CICADA. Un único pase a pantalla completa.
//
// Tres capas, como en docs/ARCHITECTURE.md:
//   impulsos   → kick, snare, hat, melodic (golpes con decaimiento)
//   energía    → energy, level (parámetros lentos)
//   estructura → escena actual y anterior con fundido

struct Uniforms {
    // ancho, alto, tiempo en segundos, ciclo
    frame: vec4<f32>,
    // kick, snare, hat, melodic
    pulses: vec4<f32>,
    // escena, escena anterior, fundido, energía
    scene: vec4<f32>,
    // destello, nivel de audio, altura de nota, avance en la sección
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

const TAU: f32 = 6.2831853;

// Escenas; deben coincidir con `scene_id` en gpu.rs.
const INTRO: i32 = 0;
const BUILD: i32 = 1;
const DROP: i32 = 2;
const BREAK: i32 = 3;

const RED: vec3<f32> = vec3<f32>(1.0, 0.08, 0.06);
const HOT: vec3<f32> = vec3<f32>(1.0, 0.55, 0.35);
const ICE: vec3<f32> = vec3<f32>(0.35, 0.55, 1.0);

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Un triángulo que cubre toda la pantalla.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), s.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), s.x),
        s.y,
    );
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var k = 0; k < 5; k++) {
        v += a * noise(q);
        q = q * 2.03 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

fn rotate(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

// Túnel de anillos: la base de la intro y del drop.
fn tunnel(p: vec2<f32>, speed: f32, sharpness: f32) -> vec2<f32> {
    let t = u.frame.z;
    let r = max(length(p), 1e-3);
    let a = atan2(p.y, p.x);
    let z = 0.32 / r + t * speed;
    let rings = pow(0.5 + 0.5 * cos(z * TAU), sharpness);
    let spokes = pow(0.5 + 0.5 * cos(a * 6.0 + z * 1.5 + u.extra.z * 4.0), sharpness * 2.0);
    let depth = smoothstep(0.02, 0.7, r);
    return vec2<f32>(rings, spokes) * depth;
}

fn scene_intro(p: vec2<f32>) -> vec3<f32> {
    let e = u.scene.w;
    let tun = tunnel(rotate(p, u.frame.z * 0.05), 0.4 + e, 10.0);
    let v = tun.x * (0.45 + 0.35 * u.pulses.x) + tun.y * 0.25 * u.pulses.w;
    return RED * v + vec3<f32>(0.02, 0.0, 0.01);
}

// Rejilla en perspectiva que acelera a medida que avanza la sección.
fn scene_build(p: vec2<f32>) -> vec3<f32> {
    let t = u.frame.z;
    let push = u.extra.w;
    var col = vec3<f32>(0.0);
    let horizon = 0.02;
    let y = abs(p.y) + horizon;
    let z = 0.35 / y;
    let speed = 1.5 + 6.0 * push * push;
    let gx = abs(fract(p.x * z * 1.2) - 0.5);
    let gz = abs(fract(z + t * speed) - 0.5);
    let line = max(smoothstep(0.06 * z, 0.0, gx * 0.35), smoothstep(0.05, 0.0, gz));
    let fade = smoothstep(0.0, 0.35, y) * (0.4 + 0.6 * push);
    col += mix(RED, HOT, push) * line * fade * (0.8 + 0.3 * u.pulses.x);
    // Resplandor del horizonte que crece hasta el drop.
    col += HOT * exp(-abs(p.y) * (30.0 - 22.0 * push)) * (0.25 + push + u.pulses.y);
    return col;
}

// Rayos, túnel agresivo y núcleo que late con el kick.
fn scene_drop(p: vec2<f32>) -> vec3<f32> {
    let t = u.frame.z;
    let k = u.pulses.x;
    let q = rotate(p, t * 0.3 + k * 0.15);
    let r = max(length(q), 1e-3);
    let a = atan2(q.y, q.x);
    let tun = tunnel(q, 1.6 + 1.5 * u.scene.w, 6.0);
    let rays = pow(abs(sin(a * 7.0 + t * 1.3 + u.extra.z * 3.0)), 24.0 - 16.0 * k);
    let core = 0.03 / r * (0.55 + 0.35 * k);
    var col = RED * (tun.x * (1.0 + 0.35 * k) + rays * 0.5 * smoothstep(0.1, 0.6, r));
    col += HOT * tun.y * (0.3 + u.pulses.w);
    col += vec3<f32>(1.0, 0.9, 0.85) * core;
    col += RED * 0.12 * fbm(q * 3.0 + t) * u.scene.w;
    return col;
}

// Niebla fría y lenta: el respiro entre drops.
fn scene_break(p: vec2<f32>) -> vec3<f32> {
    let t = u.frame.z;
    let q = rotate(p, t * 0.02);
    let f = fbm(q * 2.2 + vec2<f32>(t * 0.05, -t * 0.03));
    let g = fbm(q * 4.0 - f + t * 0.04);
    var col = ICE * pow(g, 2.2) * 0.8;
    col += vec3<f32>(0.9) * pow(g, 8.0) * (0.4 + u.pulses.w);
    return col;
}

fn scene(id: i32, p: vec2<f32>) -> vec3<f32> {
    switch id {
        case BUILD: { return scene_build(p); }
        case DROP: { return scene_drop(p); }
        case BREAK: { return scene_break(p); }
        default: { return scene_intro(p); }
    }
}

fn world(p: vec2<f32>) -> vec3<f32> {
    let fade = smoothstep(0.0, 1.0, u.scene.z);
    let now = scene(i32(u.scene.x), p);
    if (fade >= 1.0) {
        return now;
    }
    return mix(scene(i32(u.scene.y), p), now, fade);
}

@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.frame.xy;
    let t = u.frame.z;
    let kick = u.pulses.x;
    let snare = u.pulses.y;
    let energy = u.scene.w;

    let screen = (frag.xy - 0.5 * res) / res.y;
    // Kick: golpe de zoom. El kick se expresa en movimiento, no en brillo:
    // a 200 bpm un pulso de luz por kick sería un estrobo.
    var p = screen * (1.0 - 0.12 * kick);

    // Snare: bandas horizontales desplazadas (glitch).
    let band = floor(frag.y / res.y * 28.0);
    let tick = floor(t * 24.0);
    let glitch = step(0.8, hash(vec2<f32>(band, tick))) * snare;
    p.x += glitch * 0.12 * (hash(vec2<f32>(band, tick + 7.0)) - 0.5);

    // Aberración cromática que crece con el kick y la energía.
    let ca = 0.002 + 0.018 * kick * (0.3 + energy);
    let dir = normalize(p + vec2<f32>(1e-4));
    var col = vec3<f32>(
        world(p + dir * ca).r,
        world(p).g,
        world(p - dir * ca).b,
    );

    // Hat: chispas finas.
    col += vec3<f32>(1.0) * step(0.997, hash(floor(frag.xy / 3.0) + tick)) * u.pulses.z * 0.6;

    // Destello (limitado a 3 por segundo en timeline.rs).
    col += vec3<f32>(1.0, 0.92, 0.88) * u.extra.x * 0.28;

    // Nivel del audio real: la mezcla entera empuja el brillo.
    col *= 0.9 + 0.35 * u.extra.y;

    // Posprocesado: viñeta, líneas de barrido, grano, tono.
    col *= smoothstep(1.25, 0.25, length(screen));
    col *= 0.93 + 0.07 * sin(frag.y * 3.14159);
    col += (hash(frag.xy + fract(t) * 91.0) - 0.5) * 0.035;
    col = vec3<f32>(1.0) - exp(-col * 1.4);
    return vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
