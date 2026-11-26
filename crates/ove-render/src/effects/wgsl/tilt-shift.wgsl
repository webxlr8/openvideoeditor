// stylize::tilt_shift
// Disc blur of radius `r` (normalised to frame height) with golden-angle taps (stylize::disc_blur).
fn disc_blur(u: f32, v: f32, r: f32, taps: i32) -> vec3<f32> {
    if (r * info.h < 0.5) {
        return sample(u, v);
    }
    var acc = vec3<f32>(0.0);
    let j = hash21(ti(u * info.w), ti(v * info.h), 8u) * TAU;
    for (var i = 0; i < taps; i = i + 1) {
        let a = j + f32(i) * 2.399963;
        let rr = r * sqrt((f32(i) + 0.5) / f32(taps));
        acc = acc + sample(u + cos(a) * rr / info.aspect, v + sin(a) * rr);
    }
    return acc * (1.0 / f32(taps));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let q = rel(u, v, p_center());
    let d = rot(q.x, q.y, -p_angle()).y;
    let half = p_width() * 0.5;
    let k = sstep(half, half + max(p_falloff(), 0.01), abs(d));
    let r = k * p_intensity() / 100.0 * 0.02;
    let o = disc_blur(u, v, r, 16);
    // A touch of saturation sells the miniature look.
    let l = luma(o);
    return lerp3(vec3<f32>(l), o, 1.0 + 0.15 * (1.0 - k));
}
