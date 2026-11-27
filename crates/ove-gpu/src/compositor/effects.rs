//! Effects and transition masks ported to WGSL (design D2 port order: blurs first, then transitions). Each shader
//! mirrors its CPU function in `ove-render` (`effects/stylize.rs`, `effects/util.rs`, `transitions.rs`): the same
//! integer hash, the same sampling and the same 8-bit rounding, so the backends agree within the parity tolerance.
//! Effects not listed here run on the CPU for that layer.

/// Effect ids with a GPU version.
/// Chosen by measured cost (tests/costs.rs): the most expensive per frame first.
pub const PORTED: &[&str] = &["zoom-blur", "tilt-shift", "aurora", "chroma-key"];

/// Shared by every effect shader: the frame header, parameters (in contract order, packed by
/// `Params::packed`), sampling, the integer hash and the mixing loop.
const PRELUDE: &str = r#"
struct Head {
    a: vec4<f32>, // width, height, aspect, time
    b: vec4<f32>, // amount, generator (1/0), unused, unused
}
@group(0) @binding(0) var<uniform> head: Head;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<storage, read> prm: array<f32>;

const TAU: f32 = 6.2831855;

fn texel3(x: u32, y: u32) -> vec3<f32> {
    return round(textureLoad(src, vec2<i32>(i32(x), i32(y)), 0).rgb * 255.0);
}

/// Bilinear sample at normalised coordinates, clamped to the edge (`Ctx::sample`).
fn sample(u: f32, v: f32) -> vec3<f32> {
    let w = head.a.x;
    let h = head.a.y;
    let x = clamp(u * w - 0.5, 0.0, w - 1.0);
    let y = clamp(v * h - 0.5, 0.0, h - 1.0);
    let x0 = u32(x);
    let y0 = u32(y);
    let x1 = min(x0 + 1u, u32(w) - 1u);
    let y1 = min(y0 + 1u, u32(h) - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = texel3(x0, y0);
    let b = texel3(x1, y0);
    let c = texel3(x0, y1);
    let d = texel3(x1, y1);
    let top = a + (b - a) * fx;
    let bot = c + (d - c) * fx;
    return (top + (bot - top) * fy) / 255.0;
}

/// Integer hash to 0..1 (`util::hash21`).
fn hash21(x: i32, y: i32, seed: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0x00ffffffu) / 16777216.0;
}

/// Smooth value noise in 0..1 (`util::noise2`).
fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let xi = i32(floor(x));
    let yi = i32(floor(y));
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

/// Fractal noise (`util::fbm`).
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

/// `util::mix`: a + (b − a)·t (also for t outside 0..1).
fn lerp3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a + (b - a) * t;
}

fn param3(i: u32) -> vec3<f32> {
    return vec3<f32>(prm[i], prm[i + 1u], prm[i + 2u]);
}

fn smoothstep1(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / max(e1 - e0, 1e-6), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}

fn rot(x: f32, y: f32, deg: f32) -> vec2<f32> {
    let r = radians(deg);
    let s = sin(r);
    let c = cos(r);
    return vec2<f32>(x * c - y * s, x * s + y * c);
}

/// Aspect-corrected offset from a centre point (`Ctx::rel`).
fn rel(u: f32, v: f32, c: vec2<f32>) -> vec2<f32> {
    return vec2<f32>((u - c.x) * head.a.z, v - c.y);
}

fn effect(u: f32, v: f32) -> vec3<f32>;
"#;

const MAIN: &str = r#"
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let s = round(textureLoad(src, at, 0) * 255.0) / 255.0;
    let u = (f32(gid.x) + 0.5) / head.a.x;
    let v = (f32(gid.y) + 0.5) / head.a.y;
    let o = effect(u, v);
    let amount = head.b.x;
    let rgb = floor(clamp(s.rgb + (o - s.rgb) * amount, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + vec3<f32>(0.5)) / 255.0;
    var a = s.a;
    if (head.b.y > 0.5) {
        a = floor(s.a * 255.0 + (255.0 - s.a * 255.0) * amount + 0.5) / 255.0;
    }
    textureStore(out_tex, at, vec4<f32>(rgb, a));
}
"#;

/// `stylize::zoom_blur`: intensity, center.
const ZOOM_BLUR: &str = r#"
fn effect(u: f32, v: f32) -> vec3<f32> {
    let c = vec2<f32>(prm[1], prm[2]);
    let s = prm[0] / 100.0 * 0.35;
    let n = 20;
    let j = hash21(i32(u * head.a.x), i32(v * head.a.y), 4u) / f32(n);
    var acc = vec3<f32>(0.0);
    for (var i = 0; i < n; i = i + 1) {
        let k = 1.0 - s * (f32(i) / f32(n) + j);
        acc = acc + sample(c.x + (u - c.x) * k, c.y + (v - c.y) * k);
    }
    return acc * (1.0 / f32(n));
}
"#;

/// `stylize::tilt_shift` with `disc_blur`: intensity, width, falloff, angle, center.
const TILT_SHIFT: &str = r#"
fn disc_blur(u: f32, v: f32, r: f32, taps: i32) -> vec3<f32> {
    if (r * head.a.y < 0.5) {
        return sample(u, v);
    }
    var acc = vec3<f32>(0.0);
    let j = hash21(i32(u * head.a.x), i32(v * head.a.y), 8u) * TAU;
    for (var i = 0; i < taps; i = i + 1) {
        let a = j + f32(i) * 2.399963;
        let rr = r * sqrt((f32(i) + 0.5) / f32(taps));
        acc = acc + sample(u + cos(a) * rr / head.a.z, v + sin(a) * rr);
    }
    return acc * (1.0 / f32(taps));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let p = rel(u, v, vec2<f32>(prm[4], prm[5]));
    let d = rot(p.x, p.y, -prm[3]).y;
    let half = prm[1] * 0.5;
    let k = smoothstep1(half, half + max(prm[2], 0.01), abs(d));
    let r = k * prm[0] / 100.0 * 0.02;
    let o = disc_blur(u, v, r, 16);
    let l = luma(o);
    return mix(vec3<f32>(l), o, 1.0 + 0.15 * (1.0 - k));
}
"#;

/// `background::aurora`: sky, colorA, colorB, colorC, intensity, height, waviness, speed.
const AURORA: &str = r#"
fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = head.a.w * prm[15] * 0.03;
    let x = (u - 0.5) * head.a.z;
    var col = param3(0u);
    let star = hash21(i32(u * head.a.x / 2.0), i32(v * head.a.y / 2.0), 77u);
    if (star > 0.997) {
        col = col + vec3<f32>(0.5 * (1.0 - v));
    }
    let wav = prm[14];
    for (var i = 0u; i < 3u; i = i + 1u) {
        let fi = f32(i);
        let cc = param3(3u + i * 3u);
        let curtain = 0.32
            + fi * 0.09
            + sin(x * 2.3 + t * (1.0 + fi * 0.3) + fi * 2.0) * 0.08 * wav
            + (fbm(x * 1.5 + fi * 3.0, t * 0.7, 3u, 31u + i) - 0.5) * 0.25 * wav;
        let hh = 0.12 * prm[13];
        let dy = curtain - v;
        var body: f32;
        if (dy > 0.0) {
            body = exp(-dy / hh);
        } else {
            let q = dy / 0.015;
            body = exp(-(q * q));
        }
        let rays = 0.55 + 0.45 * fbm(x * 18.0 + t * 2.0 + fi * 5.0, fi, 3u, 41u + i);
        col = col + cc * (body * rays * prm[12] * 0.75);
    }
    return col;
}
"#;

/// `key::chroma_key` (written for the GPU from the start): keyColor, tolerance, softness, spill, choke,
/// matteBlack, matteWhite, edgeBlur, matteView. Writes alpha, so it has its own main.
const CHROMA_KEY: &str = r#"
fn effect(u: f32, v: f32) -> vec3<f32> {
    return vec3<f32>(0.0);
}

fn ycc(c: vec3<f32>) -> vec3<f32> {
    let y = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    return vec3<f32>(y, (c.b - y) / 1.8556, (c.r - y) / 1.5748);
}

fn norm_chroma(c: vec3<f32>) -> vec2<f32> {
    let k = ycc(c);
    let l = max(k.x, 0.1);
    return vec2<f32>(k.y / l, k.z / l);
}

/// (matte, distance) of colour `c`.
fn matte(c: vec3<f32>) -> vec2<f32> {
    let key = norm_chroma(param3(0u));
    let len = max(sqrt(key.x * key.x + key.y * key.y), 0.1);
    let n = norm_chroma(c);
    let dx = n.x - key.x;
    let dy = n.y - key.y;
    let d = sqrt(dx * dx + dy * dy) / len;
    let tol = prm[3];
    let soft = max(prm[4], 0.001);
    return vec2<f32>(smoothstep1(tol, tol + soft, d), d);
}

fn key4(u: f32, v: f32, src_a: f32) -> vec4<f32> {
    let w = head.a.x;
    let h = head.a.y;
    let s = sample(u, v);
    let md = matte(s);
    var m = md.x;
    let r = prm[9];
    if (r >= 0.5) {
        let taps = 16;
        var acc = 0.0;
        for (var i = 0; i < taps; i = i + 1) {
            let a = f32(i) * 2.399963;
            let rr = r * sqrt((f32(i) + 0.5) / f32(taps));
            acc = acc + matte(sample(u + cos(a) * rr / w, v + sin(a) * rr / h)).x;
        }
        m = acc / f32(taps);
    }
    let ch = prm[6];
    if (ch > 0.0) {
        m = clamp((m - ch) / max(1.0 - ch, 1e-3), 0.0, 1.0);
    } else {
        m = min(m / max(1.0 + ch, 1e-3), 1.0);
    }
    let lo = prm[7];
    let hi = prm[8];
    m = clamp((m - lo) / max(hi - lo, 1e-3), 0.0, 1.0);
    if (prm[10] >= 0.5) {
        return vec4<f32>(m, m, m, 1.0);
    }
    let keyc = param3(0u);
    // The key channel is green or blue (no dynamic vector writes: FXC rejects them).
    let blue = keyc.b > keyc.g;
    let tol = prm[3];
    let soft = max(prm[4], 0.001);
    let near = 1.0 - smoothstep1(tol, tol + soft * 2.0 + 1.0, md.y);
    let keyv = select(s.g, s.b, blue);
    let limit = select((s.r + s.b) * 0.5, (s.r + s.g) * 0.5, blue);
    let spill = max(keyv - limit, 0.0) * prm[5] * near;
    var out = s;
    if (blue) {
        out.b = out.b - spill;
    } else {
        out.g = out.g - spill;
    }
    let dl = spill * select(0.7152, 0.0722, blue);
    out = clamp(out + vec3<f32>(dl), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(out, src_a * m);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let s = round(textureLoad(src, at, 0) * 255.0) / 255.0;
    let u = (f32(gid.x) + 0.5) / head.a.x;
    let v = (f32(gid.y) + 0.5) / head.a.y;
    let o = key4(u, v, s.a);
    let amount = head.b.x;
    let q = floor(clamp(s + (o - s) * amount, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0 + vec4<f32>(0.5)) / 255.0;
    textureStore(out_tex, at, q);
}
"#;

/// The shader source of a ported effect.
pub fn source(id: &str) -> Option<String> {
    let body = match id {
        "zoom-blur" => ZOOM_BLUR,
        "tilt-shift" => TILT_SHIFT,
        "aurora" => AURORA,
        "chroma-key" => {
            let prelude = PRELUDE.replace("fn effect(u: f32, v: f32) -> vec3<f32>;\n", "");
            return Some(format!("{prelude}{CHROMA_KEY}"));
        }
        _ => return None,
    };
    // WGSL has no forward declarations: drop the prelude's placeholder line.
    let prelude = PRELUDE.replace("fn effect(u: f32, v: f32) -> vec3<f32>;\n", "");
    Some(format!("{prelude}{body}{MAIN}"))
}

/// Transition reveal masks (`TransState::apply_mask`): wipe (4 directions), radial wipe, ink bleed, mask glitch.
pub const MASK: &str = r#"
struct Head {
    a: vec4<f32>, // kind (0 wipe, 1 radial, 2 ink bleed, 3 mask glitch), direction (0 left, 1 right, 2 up, 3 down), q, unused
}
@group(0) @binding(0) var<uniform> head: Head;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

fn hash21(x: i32, y: i32, seed: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0x00ffffffu) / 16777216.0;
}

fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let xi = i32(floor(x));
    let yi = i32(floor(y));
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

fn smooth1(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / max(e1 - e0, 1e-6), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn sweep(x: f32, e: f32, q: f32) -> f32 {
    let th = (1.0 - q) * (1.0 + 2.0 * e) - e;
    return smooth1(th - e, th + e, x);
}

fn mask(u: f32, v: f32, aspect: f32) -> f32 {
    let q = clamp(head.a.z, 0.0, 1.0);
    let kind = u32(head.a.x);
    if (kind == 0u) {
        let dir = u32(head.a.y);
        if (dir == 0u) { return sweep(u, 0.02, q); }
        if (dir == 1u) { return sweep(1.0 - u, 0.02, q); }
        if (dir == 2u) { return sweep(v, 0.02, q); }
        return sweep(1.0 - v, 0.02, q);
    }
    if (kind == 1u) {
        let x = (u - 0.5) * aspect;
        let y = v - 0.5;
        let a = fract(atan2(x, -y) / 6.2831855 + 1.0);
        return sweep(1.0 - a, 0.01, q);
    }
    if (kind == 2u) {
        let n = fbm(u * 5.0 * aspect, v * 5.0, 4u, 7u);
        let x = (u - 0.5) * aspect;
        let y = v - 0.5;
        let centre = 1.0 - min(sqrt(x * x + y * y) / (0.5 * sqrt(aspect * aspect + 1.0)), 1.0);
        return sweep(n * 0.55 + centre * 0.45, 0.04, q);
    }
    let bx = i32(floor(u * 16.0));
    let by = i32(floor(v * 9.0 * (1.0 + floor(u * 3.0) * 0.5)));
    if (hash21(bx, by, 13u) < q) {
        return 1.0;
    }
    return 0.0;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let c = textureLoad(src, at, 0);
    let w = f32(dims.x);
    let h = f32(dims.y);
    let m = mask((f32(gid.x) + 0.5) / w, (f32(gid.y) + 0.5) / h, w / h);
    let a = floor(round(c.a * 255.0) * m + 0.5) / 255.0;
    textureStore(out_tex, at, vec4<f32>(c.rgb, a));
}
"#;
