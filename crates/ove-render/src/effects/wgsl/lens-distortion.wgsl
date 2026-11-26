// distort::lens_distortion
fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    let q = rel(u, v, ctr);
    let a = rot(q.x, q.y, -p_angle());
    let x = a.x;
    let y = a.y;
    let round_r = sqrt(x * x + y * y);
    let square_r = max(abs(x), abs(y));
    let r = round_r + (square_r - round_r) * p_roundness();
    let spread = max(p_spread(), 0.05);
    let clear = p_clearCenter() * spread * 0.5;
    let w = sstep(clear, spread, r);
    let k = 1.0 - p_bulge() * w * 0.5;
    let sw = p_swirl() * w * PI * 0.25;
    let b = rot(x * k, y * k, degrees(sw));
    let c2 = rot(b.x, b.y, p_angle());
    let s = unrel(c2.x, c2.y, ctr);
    return sample(s.x, s.y);
}
