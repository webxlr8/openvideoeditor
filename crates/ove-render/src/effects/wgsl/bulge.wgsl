// distort::bulge
fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    let q = rel(u, v, ctr);
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y);
    let rad = p_radius() * 0.5;
    if (r >= rad) {
        return sample(u, v);
    }
    let f = 1.0 - r / rad;
    var shape = 0.0;
    if (f > 0.0) {
        shape = pow(f, 1.0 + p_falloff() * 2.0);
    }
    let k = max(1.0 - p_strength() * shape * 0.85, 0.05);
    let s = unrel(x * k, y * k, ctr);
    return sample(s.x, s.y);
}
