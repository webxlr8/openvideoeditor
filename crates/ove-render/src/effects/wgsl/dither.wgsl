// stylize::dither
fn thr1(x: f32, thr: f32) -> f32 {
    if (x > thr) {
        return 1.0;
    }
    return 0.0;
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    var bayer = array<f32, 16>(0., 8., 2., 10., 12., 4., 14., 6., 3., 11., 1., 9., 15., 7., 13., 5.);
    let ps = max(p_pixelSize(), 1.0);
    let px = floor(u * info.w / ps);
    let py = floor(v * info.h / ps);
    let s = sample((px + 0.5) * ps / info.w, (py + 0.5) * ps / info.h);
    let ix = ((ti(px) % 4) + 4) % 4;
    let iy = ((ti(py) % 4) + 4) % 4;
    let b = bayer[iy * 4 + ix] / 16.0 + 1.0 / 32.0;
    let thr = p_threshold() + (b - 0.5) * p_spread();
    if (p_sourceColors() >= 0.5) {
        return vec3<f32>(thr1(s.x, thr), thr1(s.y, thr), thr1(s.z, thr));
    }
    if (luma(s) > thr) {
        return p_light();
    }
    return p_dark();
}
