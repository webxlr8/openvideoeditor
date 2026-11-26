// distort::spherize
fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    let rq = rel(u, v, ctr);
    let x = rq.x;
    let y = rq.y;
    let rad = p_radius() * 0.5;
    let r = sqrt(x * x + y * y);
    if (r >= rad) {
        return sample(u, v);
    }
    let q = r / rad;
    // Map the disc onto a hemisphere: the surface angle spreads the centre and compresses the rim.
    let sph = asin(q) / (PI * 0.5);
    let depth = p_depth();
    let nq = max(q + (sph - q) * depth, 0.0);
    var k = 1.0 - depth * 0.36;
    if (q > 1e-5) {
        k = nq / q;
    }
    let s = unrel(x * k, y * k, ctr);
    let base = sample(s.x, s.y);
    // Light the sphere from the light position (relative to the centre).
    let z = sqrt(max(1.0 - q * q, 0.0));
    let n0 = x / rad;
    let n1 = y / rad;
    let n2 = z;
    let lp = p_lightPosition();
    let lx = (lp.x - 0.5) * 2.0;
    let ly = (lp.y - 0.5) * 2.0;
    let ll = sqrt(lx * lx + ly * ly + 0.64);
    let ndl = max((n0 * lx + n1 * ly + n2 * 0.8) / ll, 0.0);
    let shin = 4.0 + (1.0 - p_lightSoftness()) * 60.0;
    var pw = 0.0;
    if (ndl > 0.0) {
        pw = pow(ndl, shin);
    }
    let spec = pw * p_lightIntensity();
    let shade = 1.0 - p_lightIntensity() * 0.35 * (1.0 - ndl);
    let lc = p_lightColor();
    return vec3<f32>(base.x * shade + lc.x * spec, base.y * shade + lc.y * spec, base.z * shade + lc.z * spec);
}
