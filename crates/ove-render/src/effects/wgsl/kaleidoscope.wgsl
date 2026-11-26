// distort::kaleidoscope
// f32::rem_euclid: the truncated remainder, moved into 0..|y|.
fn rem_euclid(x: f32, y: f32) -> f32 {
    let r = x % y;
    if (r < 0.0) {
        return r + abs(y);
    }
    return r;
}

// Fold back into the frame instead of clamping, so edges stay patterned.
fn fold(t: f32) -> f32 {
    return 1.0 - abs(fract1(t * 0.5) * 2.0 - 1.0);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    let q = rel(u, v, ctr);
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y);
    // f32::round rounds half away from zero (WGSL round is half to even); segments are positive.
    let seg = TAU / max(floor(p_segments() + 0.5), 2.0);
    let a0 = radians(p_angle());
    var a = rem_euclid(atan2(y, x) - a0, seg);
    if (a > seg * 0.5) {
        a = seg - a;
    }
    a = a + a0;
    let s = unrel(r * cos(a), r * sin(a), ctr);
    return sample(fold(s.x), fold(s.y));
}
