// background::spiral
fn effect(u: f32, v: f32) -> vec3<f32> {
    let q = rel(u, v, p_center());
    let x = q.x;
    let y = q.y;
    let r = sqrt(x * x + y * y);
    let a = atan2(y, x) / TAU;
    let s = fract1(r * p_scale() * 2.0 + a - info.t * p_speed() * 0.25);
    let d = abs(s - 0.5) * 2.0; // 0 at the stroke centre
    let width = p_strokeWidth() * (1.0 - p_strokeFalloff() * min(r * 1.5, 1.0));
    let soft = max(p_softness(), 0.005);
    let stroke = 1.0 - sstep(width - soft, width + soft, d);
    return lerp3(p_colorA(), p_colorB(), stroke);
}
