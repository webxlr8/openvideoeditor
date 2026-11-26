// stylize::tritone
fn tone3(sh: vec3<f32>, mid: vec3<f32>, hi: vec3<f32>, l: f32, m0: f32) -> vec3<f32> {
    let m = clamp(m0, 0.02, 0.98);
    if (l < m) {
        return lerp3(sh, mid, l / m);
    }
    return lerp3(mid, hi, (l - m) / (1.0 - m));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    return tone3(p_shadows(), p_midtones(), p_highlights(), luma(sample(u, v)), p_midpoint());
}
