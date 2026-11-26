// background::sunburst
// f32::round: halves away from zero (WGSL's round goes to even).
fn round_away(x: f32) -> f32 {
    let a = abs(x);
    var f = floor(a);
    if (a - f >= 0.5) {
        f = f + 1.0;
    }
    return select(f, -f, x < 0.0);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let q = rel(u, v, p_center());
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y);
    let a = atan2(y, x) + info.t * p_speed();
    let n = round_away(p_rayCount());
    let w = abs(cos(a * n * 0.5)); // 1 on a ray, 0 between rays
    let soft = max(p_softness(), 0.01) * 0.5;
    let ray = sstep(0.5 - soft, 0.5 + soft, w);
    let rad = max(p_radius(), 0.01);
    let mask = 1.0 - sstep(rad * (1.0 - p_feather()), rad, r);
    return lerp3(p_background(), p_color(), ray * mask);
}
