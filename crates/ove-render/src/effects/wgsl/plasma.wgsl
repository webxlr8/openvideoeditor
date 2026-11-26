// background::plasma
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

// powf of a non-negative base (0 stays 0 for a positive exponent, as in Rust).
fn powp(b: f32, e: f32) -> f32 {
    if (b <= 0.0) {
        return 0.0;
    }
    return pow(b, e);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed() * 0.5;
    let c0 = centred(u, v);
    let den = p_density() * 3.0 + 1.0;
    let warp = p_warp() * 2.0 * (fbm(c0.x * 2.0, c0.y * 2.0 + t * 0.2, 3u, 51u) - 0.5);
    let x = c0.x + warp;
    let y = c0.y - warp;
    let s = sin(x * den + t)
        + sin(y * den * 1.3 - t * 1.2)
        + sin((x + y) * den * 0.7 + t * 0.8)
        + sin(sqrt(x * x + y * y) * den * 2.0 - t * 1.5);
    let k0 = clamp(s * 0.125 + 0.5, 0.0, 1.0);
    let k = powp(clamp(k0 + p_balance() - 0.5, 0.0, 1.0), 1.0 / p_intensity());
    return lerp3(p_colorB(), p_colorA(), k);
}
