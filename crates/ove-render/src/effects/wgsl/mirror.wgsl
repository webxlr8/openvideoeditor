// distort::mirror
fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    let q = rel(u, v, ctr);
    var x = q.x;
    var y = q.y;
    // Mirror line through the centre with direction `angle`; reflect the negative side onto the positive.
    let n = rot(0.0, 1.0, p_angle());
    let d = x * n.x + y * n.y;
    if (d < 0.0) {
        x = q.x - 2.0 * d * n.x;
        y = q.y - 2.0 * d * n.y;
    }
    let s = unrel(x, y, ctr);
    return sample(s.x, s.y);
}
