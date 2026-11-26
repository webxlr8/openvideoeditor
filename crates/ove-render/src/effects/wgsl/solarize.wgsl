// stylize::solarize
fn sol(x: f32, th: f32) -> f32 {
    if (x > th) {
        return 1.0 - x;
    }
    return x;
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let th = p_threshold();
    let s = p_strength();
    let src = sample(u, v);
    return lerp3(src, vec3<f32>(sol(src.x, th), sol(src.y, th), sol(src.z, th)), s);
}
