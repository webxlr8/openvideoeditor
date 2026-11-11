//! WGSL sources. Conversion passes write the frame at its own resolution into an RGBA16F
//! intermediate; the present pass scales it (bilinear) into the letterboxed viewport.

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
}
@group(0) @binding(0) var<uniform> params: Params;

fn convert(y: f32, u: f32, v: f32) -> vec4f {
    let c = vec3f(
        (y - params.range.x) / params.range.y,
        (u - params.range.z) / params.range.w,
        (v - params.range.z) / params.range.w,
    );
    let rgb = vec3f(dot(params.m0.xyz, c), dot(params.m1.xyz, c), dot(params.m2.xyz, c));
    return vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), 1.0);
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
