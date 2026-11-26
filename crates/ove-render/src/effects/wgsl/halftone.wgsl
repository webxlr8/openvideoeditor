// stylize::halftone
// The ink to print at (u, v): mode 0 is 1 − luma, modes 1..4 are the C, M, Y, K separations.
fn ink(u: f32, v: f32, mode: i32) -> f32 {
    let s = sample(u, v);
    if (mode == 0) {
        return 1.0 - luma(s);
    }
    let k = 1.0 - max(max(s.x, s.y), s.z);
    let d = max(1.0 - k, 1e-3);
    if (mode == 1) {
        return (1.0 - s.x - k) / d;
    }
    if (mode == 2) {
        return (1.0 - s.y - k) / d;
    }
    if (mode == 3) {
        return (1.0 - s.z - k) / d;
    }
    return k;
}

// Dot coverage for a screen at `angle`, `freq` cells per frame height (stylize::screen).
fn screen(u: f32, v: f32, freq: f32, angle: f32, mode: i32) -> f32 {
    let p = rot((u - 0.5) * info.aspect, v - 0.5, angle);
    let gx = p.x * freq;
    let gy = p.y * freq;
    let cx = floor(gx) + 0.5;
    let cy = floor(gy) + 0.5;
    let b = rot(cx / freq, cy / freq, -angle);
    let amount = clamp(ink(b.x / info.aspect + 0.5, b.y + 0.5, mode), 0.0, 1.0);
    let r = sqrt(amount) * 0.62;
    let dx = gx - cx;
    let dy = gy - cy;
    let d = sqrt(dx * dx + dy * dy);
    let aa = max(freq / info.h, 0.02) * 1.5;
    return 1.0 - sstep(r - aa, r + aa, d);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let f = p_frequency();
    let a = p_angle();
    if (p_cmyk() < 0.5) {
        let k = screen(u, v, f, a, 0);
        return vec3<f32>(1.0 - k);
    }
    let mis = p_misprint();
    let cc = screen(u + mis, v, f, a + 15.0, 1);
    let mm = screen(u, v + mis, f, a + 75.0, 2);
    let yy = screen(u - mis, v, f, a, 3);
    let kk = screen(u, v - mis, f, a + 45.0, 4);
    return vec3<f32>((1.0 - cc) * (1.0 - kk), (1.0 - mm) * (1.0 - kk), (1.0 - yy) * (1.0 - kk));
}
