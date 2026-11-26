// stylize::zoom_blur
fn effect(u: f32, v: f32) -> vec3<f32> {
    let c = p_center();
    let s = p_intensity() / 100.0 * 0.35;
    let n = 20;
    let j = hash21(ti(u * info.w), ti(v * info.h), 4u) / f32(n);
    var acc = vec3<f32>(0.0);
    for (var i = 0; i < n; i = i + 1) {
        let k = 1.0 - s * (f32(i) / f32(n) + j);
        acc = acc + sample(c.x + (u - c.x) * k, c.y + (v - c.y) * k);
    }
    return acc * (1.0 / f32(n));
}
