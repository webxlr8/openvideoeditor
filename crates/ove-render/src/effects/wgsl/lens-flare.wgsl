// stylize::lens_flare
fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed();
    let l = p_position();
    let flick = 1.0 + 0.08 * sin(t * 7.0) * cos(t * 3.1);
    let q = rel(u, v, l);
    let x = q.x;
    let y = q.y;
    let d = sqrt(x * x + y * y);
    var add_c = vec3<f32>(0.0);
    // Core glow and starburst around the light.
    let a = atan2(y, x);
    let star = (pow(abs(cos(a * 6.0 + t * 0.3)), 40.0) + pow(abs(cos(a * 9.0 - t * 0.2)), 60.0) * 0.6)
        * exp(-d * 5.0);
    let core = exp(-d * 16.0) + star * p_starburst() * 1.5;
    add_c = add_c + vec3<f32>(1.0, 0.95, 0.85) * core;
    // Anamorphic streak along the horizontal.
    let streak = exp(-abs(y) * 220.0) * exp(-abs(x) * 1.4) * p_streak() * 1.5;
    add_c = add_c + vec3<f32>(0.6, 0.75, 1.0) * streak;
    // Ghosts on the axis through the frame centre.
    let ax = (0.5 - l.x) * info.aspect;
    let ay = 0.5 - l.y;
    var tints = array<vec3<f32>, 5>(
        vec3<f32>(0.4, 0.8, 1.0), vec3<f32>(1.0, 0.6, 0.3), vec3<f32>(0.5, 1.0, 0.6),
        vec3<f32>(0.9, 0.4, 1.0), vec3<f32>(1.0, 0.9, 0.5));
    var ks = array<f32, 5>(0.6, 1.2, 1.55, 2.1, 2.6);
    for (var i = 0; i < 5; i = i + 1) {
        let k = ks[i];
        let gx = x - ax * k;
        let gy = y - ay * k;
        let gd = sqrt(gx * gx + gy * gy);
        let rad = 0.03 + 0.025 * f32(i);
        let disc = sstep(rad, rad * 0.7, gd) * 0.25;
        add_c = add_c + tints[i] * (disc * p_ghosts());
    }
    // Halo ring around the frame centre.
    let hx = x - ax;
    let hy = y - ay;
    let hd = sqrt(hx * hx + hy * hy);
    let e = (hd - 0.35) * 30.0;
    let ring = exp(-(e * e)) * 0.35 * p_halo();
    add_c = add_c + vec3<f32>(1.0, 0.7, 0.5) * ring;
    let s = sample(u, v);
    let k = p_intensity() * flick;
    return vec3<f32>(s.x + add_c.x * k, s.y + add_c.y * k, s.z + add_c.z * k);
}
