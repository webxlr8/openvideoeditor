// stylize::light_leak
fn screen_blend(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(1.0 - (1.0 - a.x) * (1.0 - b.x), 1.0 - (1.0 - a.y) * (1.0 - b.y), 1.0 - (1.0 - a.z) * (1.0 - b.z));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed();
    let pos = p_position();
    let drift = (noise2(t * 0.3, 0.5, 7u) - 0.5) * 0.2;
    let q = rel(u, v, vec2<f32>(pos.x + drift, pos.y - drift * 0.5));
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y) / p_spread();
    let a = atan2(y, x);
    let streak = 1.0 + p_streaks() * (fbm(a * 3.0 + t * 0.2, t * 0.1, 3u, 9u) - 0.5) * 1.6;
    let glow = exp(-r * r * 2.2) * max(streak, 0.0);
    var col: vec3<f32>;
    if (r < 0.5) {
        col = lerp3(p_hotColor(), p_midColor(), r * 2.0);
    } else {
        col = lerp3(p_midColor(), p_fringeColor(), min((r - 0.5) * 1.5, 1.0));
    }
    let k = min(glow * p_intensity() * 4.0, 1.0);
    return screen_blend(sample(u, v), col * k);
}
