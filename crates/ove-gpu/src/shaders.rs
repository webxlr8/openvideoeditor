//! WGSL sources. Conversion passes write the frame at its own resolution into an RGBA16F
//! intermediate (tone-mapping HDR sources to SDR on the way); the present pass scales it (bilinear)
//! into the letterboxed viewport.

const FULLSCREEN_VS: &str = r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let t = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(t * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const CONVERT_COMMON: &str = r#"
struct Params {
    m0: vec4f,
    m1: vec4f,
    m2: vec4f,
    // luma offset, luma range, chroma midpoint, chroma range (code values)
    range: vec4f,
    // transfer (0 SDR, 1 PQ, 2 HLG), EETF: PQ signal of the source peak, target max, knee start (normalised PQ)
    tone: vec4f,
    // linear-light gamut matrix rows (BT.2020 -> BT.709 for HDR sources)
    g0: vec4f,
    g1: vec4f,
    g2: vec4f,
}
@group(0) @binding(0) var<uniform> params: Params;

const PQ_M1: f32 = 0.1593017578125;
const PQ_M2: f32 = 78.84375;
const PQ_C1: f32 = 0.8359375;
const PQ_C2: f32 = 18.8515625;
const PQ_C3: f32 = 18.6875;
const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 0.28466892;
const HLG_C: f32 = 0.55991073;
// HDR reference white (BT.2408), the HLG signal's value of it (75 %), the reference OOTF's system gamma.
const REF_WHITE: f32 = 203.0;
const HLG_REF_SCENE: f32 = 0.26496256;
const SYSTEM_GAMMA: f32 = 1.2;

// SMPTE ST 2084: signal -> cd/m2.
fn pq_eotf(e: vec3f) -> vec3f {
    let p = pow(clamp(e, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / PQ_M2));
    return 10000.0 * pow(max(p - PQ_C1, vec3f(0.0)) / (PQ_C2 - PQ_C3 * p), vec3f(1.0 / PQ_M1));
}

fn pq_inverse(nits: f32) -> f32 {
    let y = pow(clamp(nits / 10000.0, 0.0, 1.0), PQ_M1);
    return pow((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y), PQ_M2);
}

// ARIB STD-B67 inverse OETF: signal -> scene light 0..1.
fn hlg_inverse(e: vec3f) -> vec3f {
    let x = clamp(e, vec3f(0.0), vec3f(1.0));
    return select((exp((x - HLG_C) / HLG_A) + HLG_B) / 12.0, x * x / 3.0, x <= vec3f(0.5));
}

fn bt709_oetf(l: vec3f) -> vec3f {
    let x = clamp(l, vec3f(0.0), vec3f(1.0));
    return select(1.099 * pow(x, vec3f(0.45)) - 0.099, 4.5 * x, x < vec3f(0.018));
}

// ITU-R BT.2390 EETF on relative scene light (1.0 = reference white) -> SDR linear light 0..1.
fn tone_curve(m: f32) -> f32 {
    let src_pq = params.tone.y;
    let max_lum = params.tone.z;
    let ks = params.tone.w;
    let e1 = min(pq_inverse(m * REF_WHITE) / src_pq, 1.0);
    var e2 = e1;
    if (e1 >= ks) {
        let t = (e1 - ks) / (1.0 - ks);
        let t2 = t * t;
        let t3 = t2 * t;
        e2 = (2.0 * t3 - 3.0 * t2 + 1.0) * ks + (t3 - 2.0 * t2 + t) * (1.0 - ks) + (-2.0 * t3 + 3.0 * t2) * max_lum;
    }
    return min(pq_eotf(vec3f(e2 * src_pq)).x / REF_WHITE, 1.0);
}

// Non-linear HDR R'G'B' -> SDR BT.709 R'G'B' (same pipeline as ove_media::tonemap).
fn tone_map(rgb: vec3f) -> vec3f {
    var lin: vec3f;
    if (params.tone.x < 1.5) {
        lin = pow(pq_eotf(rgb) / REF_WHITE, vec3f(1.0 / SYSTEM_GAMMA));
    } else {
        lin = hlg_inverse(rgb) / HLG_REF_SCENE;
    }
    let g = max(vec3f(dot(params.g0.xyz, lin), dot(params.g1.xyz, lin), dot(params.g2.xyz, lin)), vec3f(0.0));
    let m = max(g.x, max(g.y, g.z));
    var k = 1.0;
    if (m > 0.0) {
        k = tone_curve(m) / m;
    }
    return bt709_oetf(g * k);
}

fn convert(y: f32, u: f32, v: f32) -> vec4f {
    let c = vec3f(
        (y - params.range.x) / params.range.y,
        (u - params.range.z) / params.range.w,
        (v - params.range.z) / params.range.w,
    );
    let rgb = clamp(vec3f(dot(params.m0.xyz, c), dot(params.m1.xyz, c), dot(params.m2.xyz, c)), vec3f(0.0), vec3f(1.0));
    if (params.tone.x > 0.5) {
        return vec4f(tone_map(rgb), 1.0);
    }
    return vec4f(rgb, 1.0);
}
"#;

const RGBA_FS: &str = r#"
@group(0) @binding(1) var plane0: texture_2d<f32>;
@fragment
fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return textureLoad(plane0, vec2i(p.xy), 0);
}
"#;

const NV12_FS: &str = r#"
@group(0) @binding(1) var plane0: texture_2d<f32>;
@group(0) @binding(2) var plane1: texture_2d<f32>;
@fragment
fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let xy = vec2i(p.xy);
    let y = round(textureLoad(plane0, xy, 0).r * 255.0);
    let c = round(textureLoad(plane1, xy / 2, 0).rg * 255.0);
    return convert(y, c.x, c.y);
}
"#;

const P010_FS: &str = r#"
@group(0) @binding(1) var plane0: texture_2d<u32>;
@group(0) @binding(2) var plane1: texture_2d<u32>;
@fragment
fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let xy = vec2i(p.xy);
    let y = f32(textureLoad(plane0, xy, 0).r >> 6u);
    let c = textureLoad(plane1, xy / 2, 0).rg >> vec2u(6u);
    return convert(y, f32(c.x), f32(c.y));
}
"#;

const PRESENT: &str = r#"
struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let t = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VOut;
    out.pos = vec4f(t.x * 2.0 - 1.0, 1.0 - t.y * 2.0, 0.0, 1.0);
    out.uv = t;
    return out;
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@fragment
fn fs(in: VOut) -> @location(0) vec4f {
    return textureSample(image, image_sampler, in.uv);
}
"#;

pub fn rgba_convert() -> String {
    format!("{FULLSCREEN_VS}{CONVERT_COMMON}{RGBA_FS}")
}

pub fn nv12_convert() -> String {
    format!("{FULLSCREEN_VS}{CONVERT_COMMON}{NV12_FS}")
}

pub fn p010_convert() -> String {
    format!("{FULLSCREEN_VS}{CONVERT_COMMON}{P010_FS}")
}

pub fn present() -> &'static str {
    PRESENT
}
