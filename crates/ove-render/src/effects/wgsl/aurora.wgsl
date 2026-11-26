// background::aurora
fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed() * 0.03;
    let x = (u - 0.5) * info.aspect;
    var col = p_sky();
    // Faint stars.
    let star = hash21(ti(u * info.w / 2.0), ti(v * info.h / 2.0), 77u);
    if (star > 0.997) {
        col = col + vec3<f32>(0.5 * (1.0 - v));
    }
    let cols = array<vec3<f32>, 3>(p_colorA(), p_colorB(), p_colorC());
    for (var i = 0u; i < 3u; i = i + 1u) {
        let fi = f32(i);
        let curtain = 0.32
            + fi * 0.09
            + sin(x * 2.3 + t * (1.0 + fi * 0.3) + fi * 2.0) * 0.08 * p_waviness()
            + (fbm(x * 1.5 + fi * 3.0, t * 0.7, 3u, 31u + i) - 0.5) * 0.25 * p_waviness();
        let h = 0.12 * p_height();
        let dy = curtain - v;
        var body: f32;
        if (dy > 0.0) {
            body = exp(-dy / h);
        } else {
            let q = dy / 0.015;
            body = exp(-(q * q));
        }
        let rays = 0.55 + 0.45 * fbm(x * 18.0 + t * 2.0 + fi * 5.0, fi, 3u, 41u + i);
        col = col + cols[i] * (body * rays * p_intensity() * 0.75);
    }
    return col;
}
