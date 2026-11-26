// background::conic_gradient
// f32::round: halves away from zero (WGSL's round goes to even).
fn round_away(x: f32) -> f32 {
    let a = abs(x);
    var f = floor(a);
    if (a - f >= 0.5) {
        f = f + 1.0;
    }
    return select(f, -f, x < 0.0);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let q = rel(u, v, p_center());
    let a = (atan2(q.y, q.x) + PI) / TAU + p_rotation() / 360.0;
    let k = 1.0 - abs(fract1(a * round_away(p_repeat())) * 2.0 - 1.0);
    return lerp3(p_colorA(), p_colorB(), k);
}
