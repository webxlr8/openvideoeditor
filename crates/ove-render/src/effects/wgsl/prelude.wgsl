// Shared WGSL for the GPU path of effects: bindings, sampling and maths that mirror effects/util.rs one for one, so a
// ported effect reads like its Rust original. Each effect file defines `fn effect(u: f32, v: f32) -> vec3<f32>`;
// the runner adds per-parameter accessors (`p_<name>()`) and the `main` entry point.

struct Info {
    w: f32,
    h: f32,
    aspect: f32,
    t: f32,
    amount: f32,
    generator: f32,
    pad0: f32,
    pad1: f32,
};

// Source and result: RGBA8 pixels packed little-endian in u32 (r in the low byte).
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> info: Info;
// Parameter values in the order of the effect's contract: numbers in .x, colours and points in .xyz.
@group(0) @binding(3) var<uniform> params: array<vec4<f32>, 16>;

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

fn px(x: u32, y: u32) -> vec3<f32> {
    let p = src[y * u32(info.w) + x];
    return vec3<f32>(f32(p & 0xffu), f32((p >> 8u) & 0xffu), f32((p >> 16u) & 0xffu));
}

// Bilinear sample at normalised coordinates (clamped to the edge), RGB in 0..1 (Ctx::sample).
fn sample(u: f32, v: f32) -> vec3<f32> {
    let x = clamp(u * info.w - 0.5, 0.0, info.w - 1.0);
    let y = clamp(v * info.h - 0.5, 0.0, info.h - 1.0);
    let x0 = u32(x);
    let y0 = u32(y);
    let x1 = min(x0 + 1u, u32(info.w) - 1u);
    let y1 = min(y0 + 1u, u32(info.h) - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let top = px(x0, y0) + (px(x1, y0) - px(x0, y0)) * fx;
    let bot = px(x0, y1) + (px(x1, y1) - px(x0, y1)) * fx;
    return (top + (bot - top) * fy) / 255.0;
}

// Aspect-corrected offset from a centre point (Ctx::rel) and its inverse (Ctx::abs).
fn rel(u: f32, v: f32, c: vec2<f32>) -> vec2<f32> {
    return vec2<f32>((u - c.x) * info.aspect, v - c.y);
}
fn unrel(x: f32, y: f32, c: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(x / info.aspect + c.x, y + c.y);
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

// a + (b − a)·t, written as in util.rs (WGSL's mix rounds differently).
fn lerp3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a + (b - a) * t;
}
fn lerp1(a: f32, b: f32, t: f32) -> f32 {
    return a + (b - a) * t;
}

fn sstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / max(e1 - e0, 1e-6), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn fract1(x: f32) -> f32 {
    return x - floor(x);
}

fn rot(x: f32, y: f32, deg: f32) -> vec2<f32> {
    let r = radians(deg);
    let s = sin(r);
    let c = cos(r);
    return vec2<f32>(x * c - y * s, x * s + y * c);
}

// Integer hash to 0..1 (util::hash21; u32 multiplication wraps in WGSL as wrapping_mul does in Rust).
fn hash21(x: i32, y: i32, seed: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0x00ffffffu) / 16777216.0;
}

// `as i32` of a float in Rust truncates towards zero (and saturates); WGSL's i32() of a finite in-range value matches.
fn ti(x: f32) -> i32 {
    return i32(clamp(x, -2147483520.0, 2147483520.0));
}

fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let xi = ti(floor(x));
    let yi = ti(floor(y));
    let fx = x - f32(xi);
    let fy = y - f32(yi);
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash21(xi, yi, seed);
    let b = hash21(xi + 1, yi, seed);
    let c = hash21(xi, yi + 1, seed);
    let d = hash21(xi + 1, yi + 1, seed);
    let top = a + (b - a) * ux;
    let bot = c + (d - c) * ux;
    return top + (bot - top) * uy;
}

fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var o = 0u; o < octaves; o = o + 1u) {
        sum = sum + amp * noise2(x * freq, y * freq, seed + o * 101u);
        norm = norm + amp;
        amp = amp * 0.5;
        freq = freq * 2.02;
    }
    return sum / max(norm, 1e-6);
}

fn finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}
