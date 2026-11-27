//! WGSL compute shaders of the GPU compositor. Each mirrors the CPU reference in `ove-render` (`canvas.rs`,
//! `color.rs`, `backend.rs`) formula by formula, including where it rounds to 8 bits, so the two backends agree
//! within the parity tolerance. Images are `rgba8unorm`; maths is f32.

/// Shared helpers: 8-bit rounding as the CPU does it (`(x·255 + 0.5)` clamped, then truncated).
const COMMON: &str = r#"
fn q8(x: f32) -> f32 {
    return floor(clamp(x * 255.0 + 0.5, 0.0, 255.0)) / 255.0;
}
fn q8v(x: vec3<f32>) -> vec3<f32> {
    return floor(clamp(x * 255.0 + vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(255.0))) / 255.0;
}
/// An 8-bit texel as its integer code values (0..255) in f32.
fn codes(c: vec4<f32>) -> vec4<f32> {
    return round(c * 255.0);
}
"#;

/// Layer drawing: inverse affine mapping, crop, rounded corners, bilinear sampling, opacity and the 16 blend modes
/// (W3C Compositing and Blending Level 1 formulas, as `canvas::blend_ch` / `blend_rgb` / `put`).
const DRAW: &str = r#"
struct Params {
    place: vec4<f32>,  // cx, cy, sx, sy (destination pixels; negative sx mirrors)
    rot: vec4<f32>,    // cos, sin, source width, source height
    crop: vec4<f32>,   // left, right, top, bottom edges in source pixels
    misc: vec4<f32>,   // corner radius (source pixels), opacity, blend mode, unused
    bbox: vec4<u32>,   // x0, y0, x1, y1 (destination pixels touched)
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var dst_in: texture_2d<f32>;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var out_tex: texture_storage_2d<rgba8unorm, write>;
// Per row of the box, the pixels [x0, x1) inside the cropped source, decided on the CPU in f64 exactly as the
// reference does, so hard edges fall on the same pixels.
@group(0) @binding(4) var<storage, read> spans: array<vec2<u32>>;

fn texel(x: u32, y: u32) -> vec4<f32> {
    return codes(textureLoad(src, vec2<i32>(i32(x), i32(y)), 0));
}

fn sample(u: f32, v: f32) -> vec4<f32> {
    let dims = textureDimensions(src);
    let x = clamp(u - 0.5, 0.0, f32(dims.x - 1u));
    let y = clamp(v - 0.5, 0.0, f32(dims.y - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, dims.x - 1u);
    let y1 = min(y0 + 1u, dims.y - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = texel(x0, y0);
    let b = texel(x1, y0);
    let c = texel(x0, y1);
    let d = texel(x1, y1);
    let top = a + (b - a) * fx;
    let bot = c + (d - c) * fx;
    return (top + (bot - top) * fy) / 255.0;
}

fn hard_light(d: f32, s: f32) -> f32 {
    if (s <= 0.5) {
        return d * 2.0 * s;
    }
    return 1.0 - (1.0 - d) * (1.0 - (2.0 * s - 1.0));
}

fn blend_ch(m: u32, d: f32, s: f32) -> f32 {
    switch m {
        case 1u: { return d * s; }
        case 2u: { return 1.0 - (1.0 - d) * (1.0 - s); }
        case 3u: { return hard_light(s, d); }
        case 4u: { return min(d, s); }
        case 5u: { return max(d, s); }
        case 6u: {
            if (d <= 0.0) { return 0.0; }
            if (s >= 1.0) { return 1.0; }
            return min(d / (1.0 - s), 1.0);
        }
        case 7u: {
            if (d >= 1.0) { return 1.0; }
            if (s <= 0.0) { return 0.0; }
            return 1.0 - min((1.0 - d) / s, 1.0);
        }
        case 8u: { return hard_light(d, s); }
        case 9u: {
            if (s <= 0.5) {
                return d - (1.0 - 2.0 * s) * d * (1.0 - d);
            }
            var g: f32;
            if (d <= 0.25) {
                g = ((16.0 * d - 12.0) * d + 4.0) * d;
            } else {
                g = sqrt(d);
            }
            return d + (2.0 * s - 1.0) * (g - d);
        }
        case 10u: { return abs(d - s); }
        case 11u: { return d + s - 2.0 * d * s; }
        case 16u: { return min(d + s, 1.0); }
        default: { return s; }
    }
}

fn lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    var o = c;
    if (n < 0.0 && l - n > 1e-6) {
        o = vec3<f32>(l) + (o - vec3<f32>(l)) * l / (l - n);
    }
    if (x > 1.0 && x - l > 1e-6) {
        o = vec3<f32>(l) + (o - vec3<f32>(l)) * (1.0 - l) / (x - l);
    }
    return o;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = l - lum(c);
    return clip_color(c + vec3<f32>(d));
}

fn sat_of(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    if (x - n <= 1e-6) {
        return vec3<f32>(0.0);
    }
    return (c - vec3<f32>(n)) * s / (x - n);
}

fn blend_rgb(m: u32, d: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    switch m {
        case 12u: { return set_lum(set_sat(s, sat_of(d)), lum(d)); }
        case 13u: { return set_lum(set_sat(d, sat_of(s)), lum(d)); }
        case 14u: { return set_lum(s, lum(d)); }
        case 15u: { return set_lum(d, lum(s)); }
        default: {
            return vec3<f32>(blend_ch(m, d.r, s.r), blend_ch(m, d.g, s.g), blend_ch(m, d.b, s.b));
        }
    }
}

fn put(dst: vec4<f32>, s: vec3<f32>, a: f32, m: u32) -> vec4<f32> {
    if (a <= 0.0) {
        return dst;
    }
    let da = dst.a;
    let oa = a + da * (1.0 - a);
    let backdrop = dst.rgb;
    var blended = s;
    if (m != 0u) {
        blended = blend_rgb(m, backdrop, s);
    }
    let b = blended * da + s * (1.0 - da);
    let col = (backdrop * da * (1.0 - a) + b * a) / max(oa, 1e-6);
    return vec4<f32>(q8v(col), q8(oa));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(dst_in);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    var d = codes(textureLoad(dst_in, at, 0)) / 255.0;
    if (gid.x >= p.bbox.x && gid.x < p.bbox.z && gid.y >= p.bbox.y && gid.y < p.bbox.w) {
        let dx = f32(gid.x) + 0.5 - p.place.x;
        let dy = f32(gid.y) + 0.5 - p.place.y;
        let rx = dx * p.rot.x + dy * p.rot.y;
        let ry = -dx * p.rot.y + dy * p.rot.x;
        let u = rx / p.place.z + p.rot.z / 2.0;
        let v = ry / p.place.w + p.rot.w / 2.0;
        let span = spans[gid.y - p.bbox.y];
        if (gid.x >= span.x && gid.x < span.y) {
            var cover = 1.0;
            let radius = p.misc.x;
            if (radius > 0.0) {
                var cxr = u;
                if (u < p.crop.x + radius) { cxr = p.crop.x + radius; } else if (u > p.crop.y - radius) { cxr = p.crop.y - radius; }
                var cyr = v;
                if (v < p.crop.z + radius) { cyr = p.crop.z + radius; } else if (v > p.crop.w - radius) { cyr = p.crop.w - radius; }
                let dist = sqrt((u - cxr) * (u - cxr) + (v - cyr) * (v - cyr));
                cover = clamp(radius - dist + 0.5, 0.0, 1.0);
            }
            if (cover > 0.0) {
                let s = sample(u, v);
                d = put(d, s.rgb, s.a * p.misc.y * cover, u32(p.misc.z));
            }
        }
    }
    textureStore(out_tex, at, d);
}
"#;

/// The basic adjustments (`canvas::grade`): per-channel tone tables, then saturation and vibrance.
const GRADE: &str = r#"
struct Params {
    a: vec4<f32>, // saturation factor, vibrance, unused, unused
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<storage, read> tables: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let c = codes(textureLoad(src, at, 0));
    var rgb = vec3<f32>(f32(tables[u32(c.r)]), f32(tables[256u + u32(c.g)]), f32(tables[512u + u32(c.b)]));
    let sat = p.a.x;
    let vib = p.a.y;
    if (sat != 1.0 || vib != 0.0) {
        let l = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
        let mx = max(rgb.r, max(rgb.g, rgb.b));
        let mn = min(rgb.r, min(rgb.g, rgb.b));
        var cur = 0.0;
        if (mx > 0.0) {
            cur = (mx - mn) / mx;
        }
        let s = sat * (1.0 + vib * (1.0 - cur));
        rgb = vec3<f32>(l) + (rgb - vec3<f32>(l)) * s;
    }
    let o = floor(clamp(rgb + vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(255.0))) / 255.0;
    textureStore(out_tex, at, vec4<f32>(o, c.a / 255.0));
}
"#;

/// A LUT or baked grade lattice (`color::Lut::apply` with tetrahedral interpolation, `color::apply_lut`).
const LUT: &str = r#"
struct Params {
    dmin: vec4<f32>,
    dmax: vec4<f32>,
    a: vec4<f32>, // size, 3D (1) or 1D (0), intensity, unused
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<storage, read> lat: array<vec4<f32>>;

fn at3(i: u32, j: u32, l: u32, s: u32) -> vec3<f32> {
    return lat[l * s * s + j * s + i].rgb;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let k = codes(textureLoad(src, at, 0));
    let c = k.rgb / 255.0;
    let n = clamp((c - p.dmin.rgb) / (p.dmax.rgb - p.dmin.rgb), vec3<f32>(0.0), vec3<f32>(1.0));
    let size = u32(p.a.x);
    let m = f32(size - 1u);
    var o = vec3<f32>(0.0);
    if (p.a.y < 0.5) {
        for (var ch = 0u; ch < 3u; ch = ch + 1u) {
            let x = n[ch] * m;
            let i = min(u32(x), size - 2u);
            let f = x - f32(i);
            o[ch] = lat[i][ch] + (lat[i + 1u][ch] - lat[i][ch]) * f;
        }
    } else {
        let x = n.r * m;
        let y = n.g * m;
        let z = n.b * m;
        let i = min(u32(x), size - 2u);
        let j = min(u32(y), size - 2u);
        let l = min(u32(z), size - 2u);
        let fx = x - f32(i);
        let fy = y - f32(j);
        let fz = z - f32(l);
        let c000 = at3(i, j, l, size);
        let c111 = at3(i + 1u, j + 1u, l + 1u, size);
        var a: vec3<f32>;
        var b: vec3<f32>;
        var wa: f32;
        var wb: f32;
        var wc: f32;
        if (fx >= fy) {
            if (fy >= fz) {
                a = at3(i + 1u, j, l, size); b = at3(i + 1u, j + 1u, l, size); wa = fx - fy; wb = fy - fz; wc = fz;
            } else if (fx >= fz) {
                a = at3(i + 1u, j, l, size); b = at3(i + 1u, j, l + 1u, size); wa = fx - fz; wb = fz - fy; wc = fy;
            } else {
                a = at3(i, j, l + 1u, size); b = at3(i + 1u, j, l + 1u, size); wa = fz - fx; wb = fx - fy; wc = fy;
            }
        } else if (fz >= fy) {
            a = at3(i, j, l + 1u, size); b = at3(i, j + 1u, l + 1u, size); wa = fz - fy; wb = fy - fx; wc = fx;
        } else if (fz >= fx) {
            a = at3(i, j + 1u, l, size); b = at3(i, j + 1u, l + 1u, size); wa = fy - fz; wb = fz - fx; wc = fx;
        } else {
            a = at3(i, j + 1u, l, size); b = at3(i + 1u, j + 1u, l, size); wa = fy - fx; wb = fx - fz; wc = fz;
        }
        let w0 = 1.0 - wa - wb - wc;
        o = c000 * w0 + a * wa + b * wb + c111 * wc;
    }
    let v = c + (o - c) * p.a.z;
    let q = floor(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + vec3<f32>(0.5)) / 255.0;
    textureStore(out_tex, at, vec4<f32>(q, k.a / 255.0));
}
"#;

/// `dst = before + (dst − before)·k` on all four channels (`backend::mix_back`).
const MIX: &str = r#"
struct Params {
    a: vec4<f32>, // k
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var dst_in: texture_2d<f32>;
@group(0) @binding(2) var before: texture_2d<f32>;
@group(0) @binding(3) var out_tex: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(dst_in);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let d = codes(textureLoad(dst_in, at, 0));
    let b = codes(textureLoad(before, at, 0));
    let v = floor(clamp(b + (d - b) * p.a.x + vec4<f32>(0.5), vec4<f32>(0.0), vec4<f32>(255.0)));
    textureStore(out_tex, at, v / 255.0);
}
"#;

/// Alpha to 1, colour kept.
const OPAQUE: &str = r#"
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let c = textureLoad(src, at, 0);
    textureStore(out_tex, at, vec4<f32>(c.rgb, 1.0));
}
"#;

pub fn draw() -> String {
    format!("{COMMON}{DRAW}")
}

pub fn grade() -> String {
    format!("{COMMON}{GRADE}")
}

pub fn lut() -> String {
    format!("{COMMON}{LUT}")
}

pub fn mix() -> String {
    format!("{COMMON}{MIX}")
}

pub fn opaque() -> String {
    format!("{COMMON}{OPAQUE}")
}
