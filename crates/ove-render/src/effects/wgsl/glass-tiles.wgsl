// distort::glass_tiles
fn effect(u: f32, v: f32) -> vec3<f32> {
    let a = rot((u - 0.5) * info.aspect, v - 0.5, -p_rotation());
    let x = a.x;
    let y = a.y;
    let n = p_tileCount();
    let gx = x * n;
    let gy = y * n;
    let lx = fract1(gx) - 0.5;
    let ly = fract1(gy) - 0.5;
    // Each tile is a small lens: content is shifted outward from the tile centre.
    let k = p_intensity() * 0.02;
    let sr = rot(x + lx * k, y + ly * k, p_rotation());
    let col = sample(sr.x / info.aspect + 0.5, sr.y + 0.5);
    let round = p_roundness();
    var edge = 0.0;
    if (round > 0.0) {
        let r = 0.5 * round;
        let dx = max(abs(lx) - 0.5 + r, 0.0);
        let dy = max(abs(ly) - 0.5 + r, 0.0);
        edge = sstep(r - 0.04, r + 0.02, sqrt(dx * dx + dy * dy));
    }
    let bevel = 1.0 - 0.18 * sstep(0.38, 0.5, max(abs(lx), abs(ly))) - 0.7 * edge;
    return col * bevel;
}
