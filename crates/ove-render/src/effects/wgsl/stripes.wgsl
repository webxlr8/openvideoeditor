// background::stripes
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let c0 = centred(u, v);
    let x = rot(c0.x, c0.y, -p_angle()).x;
    let s = fract1(x * p_density() + info.t * p_speed());
    let b = p_balance();
    let soft = max(p_softness(), 0.002);
    // Two soft edges per period: rising at 0, falling at the balance point.
    let k = sstep(-soft, soft, min(s, 1.0 - s)) * (1.0 - sstep(b - soft, b + soft, s));
    return lerp3(p_colorA(), p_colorB(), k);
}
