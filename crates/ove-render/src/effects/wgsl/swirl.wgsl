// background::swirl
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

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
    let t = info.t * p_speed() * 0.1;
    let c0 = centred(u, v);
    let r = sqrt(c0.x * c0.x + c0.y * c0.y);
    let q = rot(c0.x, c0.y, degrees(max(1.0 - r, 0.0) * 3.0 + t));
    let oct = 1u + u32(max(round_away(p_detail()), 0.0));
    let n = fbm(q.x * 3.0 + t, q.y * 3.0, oct, 61u);
    let b = p_blend();
    return lerp3(p_colorA(), p_colorB(), sstep(0.5 - b * 0.5, 0.5 + b * 0.5, n));
}
