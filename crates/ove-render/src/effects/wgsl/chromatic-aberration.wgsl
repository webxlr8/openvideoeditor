// stylize::chromatic_aberration
fn ca_at(u: f32, v: f32, d: vec2<f32>, o: f32, k: f32) -> vec2<f32> {
    return vec2<f32>(u + d.x * o * k / info.aspect, v + d.y * o * k);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let d = rot(1.0, 0.0, p_angle());
    // Stronger towards the frame edges, as with a real lens.
    let ex = (u - 0.5) * info.aspect;
    let ey = v - 0.5;
    let r = sqrt(ex * ex + ey * ey);
    let k = p_strength() * 0.02 * (0.35 + r * 1.3);
    let r0 = ca_at(u, v, d, p_redOffset(), k);
    let g0 = ca_at(u, v, d, p_greenOffset(), k);
    let b0 = ca_at(u, v, d, p_blueOffset(), k);
    return vec3<f32>(sample(r0.x, r0.y).x, sample(g0.x, g0.y).y, sample(b0.x, b0.y).z);
}
