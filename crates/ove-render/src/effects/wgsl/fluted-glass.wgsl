// distort::fluted_glass
// The closure `at` of the Rust: sample with the flute offset scaled by k.
fn at(x: f32, y: f32, off: f32, k: f32) -> vec3<f32> {
    let r = rot(x + off * k, y, p_angle());
    return sample(r.x / info.aspect + 0.5, r.y + 0.5);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let a = rot((u - 0.5) * info.aspect, v - 0.5, -p_angle());
    let x = a.x;
    let y = a.y;
    let f = p_frequency();
    let s = x * f + info.t * p_speed();
    let local = fract1(s) - 0.5; // -0.5..0.5 across one flute
    let soft = 0.5 + p_softness() * 1.5;
    let base = max(1.0 - abs(local) * 2.0, 0.0);
    var pw = 0.0;
    if (base > 0.0) {
        pw = pow(base, 1.0 / soft);
    }
    let profile = sin(local * PI) * pw;
    let off = -profile * p_refraction() * 0.08 / max(f, 1.0);
    let ab = p_aberration() * 0.25;
    let hl = max(1.0 - abs(local * 2.0 - 0.25) * 4.0, 0.0) * p_highlight() * 0.5;
    let col = vec3<f32>(at(x, y, off, 1.0 - ab).x, at(x, y, off, 1.0).y, at(x, y, off, 1.0 + ab).z);
    return vec3<f32>(col.x + hl, col.y + hl, col.z + hl);
}
