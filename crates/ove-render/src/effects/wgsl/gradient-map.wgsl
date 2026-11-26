// stylize::gradient_map
// Triangle wave with period 2: 0→0, 1→1, 2→0 (stylize::tri).
fn tri(x: f32) -> f32 {
    return 1.0 - abs(fract1(x * 0.5) * 2.0 - 1.0);
}

fn tone3(sh: vec3<f32>, mid: vec3<f32>, hi: vec3<f32>, l: f32, m0: f32) -> vec3<f32> {
    let m = clamp(m0, 0.02, 0.98);
    if (l < m) {
        return lerp3(sh, mid, l / m);
    }
    return lerp3(mid, hi, (l - m) / (1.0 - m));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let src = sample(u, v);
    var l = (luma(src) - 0.5) * p_contrast() + 0.5;
    let bp = p_blackPoint();
    let wp = max(p_whitePoint(), p_blackPoint() + 1e-3);
    l = clamp((l - bp) / (wp - bp), 0.0, 1.0);
    let l2 = tri(l + p_speed() * info.t);
    return lerp3(src, tone3(p_shadows(), p_midtones(), p_highlights(), l2, 0.5), p_strength());
}
