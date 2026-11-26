// stylize::engraving
// One set of engraved lines at `angle`; lines appear where `dark` exceeds `level`.
fn hatch(u: f32, v: f32, angle: f32, level: f32, dark: f32, gx: f32, gy: f32) -> f32 {
    let p = rot((u - 0.5) * info.aspect, v - 0.5, angle);
    let wav = sin(p.x * 9.0) * p_waviness() * 0.35 / p_frequency() * 10.0;
    let rl = (gx + gy) * p_relief() * 4.0 / p_frequency() * 10.0;
    let ph = (p.y + wav + rl) * p_frequency();
    let d = abs(fract1(ph) - 0.5) * 2.0;
    let width = clamp((dark - level) / (1.0 - level), 0.0, 1.0);
    let aa = clamp(p_frequency() / info.h * 2.0, 0.02, 0.5);
    return 1.0 - sstep(width - aa, width + aa, d);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let s = sample(u, v);
    let l = clamp((luma(s) - 0.5) * p_contrast() + 0.5, 0.0, 1.0);
    let dark = 1.0 - l;
    let e = 1.5 / info.h;
    let gy = luma(sample(u, v + e)) - luma(sample(u, v - e));
    let gx = luma(sample(u + e, v)) - luma(sample(u - e, v));
    var ink = hatch(u, v, p_angle(), 0.0, dark, gx, gy);
    if (p_crossHatch() >= 0.5) {
        ink = max(ink, hatch(u, v, p_angle() + 90.0, 0.5, dark, gx, gy));
    }
    return lerp3(p_paperColor(), p_inkColor(), ink);
}
