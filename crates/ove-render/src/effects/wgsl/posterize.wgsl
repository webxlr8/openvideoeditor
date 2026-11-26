// stylize::posterize
// Rust f32::round: halves away from zero (WGSL round goes to even).
fn rround(x: f32) -> f32 {
    let t = trunc(x);
    if (abs(x - t) >= 0.5) {
        return t + sign(x);
    }
    return t;
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let k = max(rround(p_levels()), 2.0) - 1.0;
    let s = sample(u, v);
    return vec3<f32>(rround(s.x * k) / k, rround(s.y * k) / k, rround(s.z * k) / k);
}
