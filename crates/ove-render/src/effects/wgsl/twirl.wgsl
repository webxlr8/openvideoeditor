// distort::twirl
fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    let q = rel(u, v, ctr);
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y);
    let f = max(1.0 - r / 0.6, 0.0);
    let a = rot(x, y, degrees(p_intensity() * f * f * PI));
    let s = unrel(a.x, a.y, ctr);
    return sample(s.x, s.y);
}
