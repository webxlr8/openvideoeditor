// background::godrays
fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed() * 0.3;
    let q = rel(u, v, p_center());
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y);
    let a = atan2(y, x);
    let den = 4.0 + p_density() * 40.0;
    let rays0 = noise2(a * den, t, 71u) * 0.6 + noise2(a * den * 2.3, -t * 1.3, 72u) * 0.4;
    let rays = sstep(0.35, 0.9, rays0);
    let spots = 1.0 - p_spotty() * 0.7 * (1.0 - fbm(r * 6.0 - t * 2.0, a * 3.0, 3u, 73u));
    let fall = exp(-r * 1.2);
    let k = clamp(rays * spots * fall * p_intensity() * 1.6, 0.0, 1.0);
    return lerp3(p_background(), p_rayColor(), k);
}
