//! Stylize effects: colour mapping, print/screen looks, optical light. Each is a per-pixel function of the source.

use std::f32::consts::{PI, TAU};

use super::util::*;

/// Triangle wave with period 2: 0→0, 1→1, 2→0 (seamless cycling of a 0..1 ramp).
fn tri(x: f32) -> f32 {
    1.0 - (fract(x * 0.5) * 2.0 - 1.0).abs()
}

fn tone3(sh: V3, mid: V3, hi: V3, l: f32, m: f32) -> V3 {
    let m = m.clamp(0.02, 0.98);
    if l < m { mix(sh, mid, l / m) } else { mix(mid, hi, (l - m) / (1.0 - m)) }
}

pub fn duotone(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let l = luma(c.sample(u, v));
    let t = smoothstep(0.0, 1.0, (l + 0.5 - p.n("balance")).clamp(0.0, 1.0));
    mix(p.c("shadows"), p.c("highlights"), t)
}

pub fn tritone(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    tone3(p.c("shadows"), p.c("midtones"), p.c("highlights"), luma(c.sample(u, v)), p.n("midpoint"))
}

pub fn gradient_map(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let src = c.sample(u, v);
    let mut l = (luma(src) - 0.5) * p.n("contrast") + 0.5;
    let (bp, wp) = (p.n("blackPoint"), p.n("whitePoint").max(p.n("blackPoint") + 1e-3));
    l = ((l - bp) / (wp - bp)).clamp(0.0, 1.0);
    let l = tri(l + p.n("speed") * c.t);
    mix(src, tone3(p.c("shadows"), p.c("midtones"), p.c("highlights"), l, 0.5), p.n("strength"))
}

pub fn posterize(c: &Ctx, u: f32, v: f32) -> V3 {
    let k = c.p.n("levels").round().max(2.0) - 1.0;
    c.sample(u, v).map(|x| (x * k).round() / k)
}

pub fn solarize(c: &Ctx, u: f32, v: f32) -> V3 {
    let (th, s) = (c.p.n("threshold"), c.p.n("strength"));
    let src = c.sample(u, v);
    mix(src, src.map(|x| if x > th { 1.0 - x } else { x }), s)
}

/// Dot coverage for a screen at `angle`, `freq` cells per frame height; `ink` is the 0..1 amount to print.
fn screen(c: &Ctx, u: f32, v: f32, freq: f32, angle: f32, ink: &dyn Fn(f32, f32) -> f32) -> f32 {
    let (x, y) = rot((u - 0.5) * c.aspect, v - 0.5, angle);
    let (gx, gy) = (x * freq, y * freq);
    let (cx, cy) = (gx.floor() + 0.5, gy.floor() + 0.5);
    // Cell centre back in frame coordinates, to read how much ink this cell carries.
    let (bx, by) = rot(cx / freq, cy / freq, -angle);
    let amount = ink(bx / c.aspect + 0.5, by + 0.5).clamp(0.0, 1.0);
    let r = amount.sqrt() * 0.62;
    let d = ((gx - cx).powi(2) + (gy - cy).powi(2)).sqrt();
    let aa = (freq / c.h).max(0.02) * 1.5;
    1.0 - smoothstep(r - aa, r + aa, d)
}

pub fn halftone(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (f, a) = (p.n("frequency"), p.n("angle"));
    if p.n("cmyk") < 0.5 {
        let k = screen(c, u, v, f, a, &|u, v| 1.0 - luma(c.sample(u, v)));
        return [1.0 - k; 3];
    }
    let mis = p.n("misprint");
    let cmyk = |u: f32, v: f32| {
        let s = c.sample(u, v);
        let k = 1.0 - s[0].max(s[1]).max(s[2]);
        let d = (1.0 - k).max(1e-3);
        [(1.0 - s[0] - k) / d, (1.0 - s[1] - k) / d, (1.0 - s[2] - k) / d, k]
    };
    let cc = screen(c, u + mis, v, f, a + 15.0, &|u, v| cmyk(u, v)[0]);
    let mm = screen(c, u, v + mis, f, a + 75.0, &|u, v| cmyk(u, v)[1]);
    let yy = screen(c, u - mis, v, f, a, &|u, v| cmyk(u, v)[2]);
    let kk = screen(c, u, v - mis, f, a + 45.0, &|u, v| cmyk(u, v)[3]);
    [(1.0 - cc) * (1.0 - kk), (1.0 - mm) * (1.0 - kk), (1.0 - yy) * (1.0 - kk)]
}

const BAYER4: [f32; 16] = [0., 8., 2., 10., 12., 4., 14., 6., 3., 11., 1., 9., 15., 7., 13., 5.];

pub fn dither(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ps = p.n("pixelSize").max(1.0);
    let (px, py) = ((u * c.w / ps).floor(), (v * c.h / ps).floor());
    let s = c.sample((px + 0.5) * ps / c.w, (py + 0.5) * ps / c.h);
    let b = BAYER4[((py as i64).rem_euclid(4) * 4 + (px as i64).rem_euclid(4)) as usize] / 16.0 + 1.0 / 32.0;
    let thr = p.n("threshold") + (b - 0.5) * p.n("spread");
    if p.n("sourceColors") >= 0.5 {
        return s.map(|x| if x > thr { 1.0 } else { 0.0 });
    }
    if luma(s) > thr { p.c("light") } else { p.c("dark") }
}

pub fn engraving(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let s = c.sample(u, v);
    let l = ((luma(s) - 0.5) * p.n("contrast") + 0.5).clamp(0.0, 1.0);
    let dark = 1.0 - l;
    // Relief: lines bend with the local brightness slope, like a burin following the form.
    let e = 1.5 / c.h;
    let gy = luma(c.sample(u, v + e)) - luma(c.sample(u, v - e));
    let gx = luma(c.sample(u + e, v)) - luma(c.sample(u - e, v));
    let hatch = |angle: f32, level: f32| {
        let (x, y) = rot((u - 0.5) * c.aspect, v - 0.5, angle);
        let wav = (x * 9.0).sin() * p.n("waviness") * 0.35 / p.n("frequency") * 10.0;
        let rel = (gx + gy) * p.n("relief") * 4.0 / p.n("frequency") * 10.0;
        let ph = (y + wav + rel) * p.n("frequency");
        let d = (fract(ph) - 0.5).abs() * 2.0; // 0 at the line centre, 1 between lines
        let width = ((dark - level) / (1.0 - level)).clamp(0.0, 1.0);
        let aa = (p.n("frequency") / c.h * 2.0).clamp(0.02, 0.5);
        1.0 - smoothstep(width - aa, width + aa, d)
    };
    let mut ink = hatch(p.n("angle"), 0.0);
    if p.n("crossHatch") >= 0.5 {
        ink = ink.max(hatch(p.n("angle") + 90.0, 0.5));
    }
    mix(p.c("paperColor"), p.c("inkColor"), ink)
}

pub fn watercolor(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let r = p.n("brushSize") * 2.0;
    // Soft wash: average over a jittered disc, then a gentle posterize to form pigment pools.
    let mut acc = [0.0; 3];
    let n = 8;
    let j = noise2(u * c.w / 7.0, v * c.h / 7.0, 11) * TAU;
    for i in 0..n {
        let a = j + i as f32 * 2.399_963;
        let rr = r * ((i as f32 + 0.5) / n as f32).sqrt();
        acc = add(acc, c.sample(u + a.cos() * rr / c.w, v + a.sin() * rr / c.h));
    }
    let wash = scale(acc, 1.0 / n as f32).map(|x| ((x * 6.0).round() / 6.0) * 0.6 + x * 0.4);
    // Edge darkening where pigment pools (bleed), by the local contrast.
    let e = 1.5 / c.h;
    let g = (luma(c.sample(u + e * c.h / c.w, v)) - luma(c.sample(u - e * c.h / c.w, v))).abs()
        + (luma(c.sample(u, v + e)) - luma(c.sample(u, v - e))).abs();
    let pooled = scale(wash, 1.0 - (g * p.n("bleed") * 1.2).min(0.35));
    // Paper grain and white paper showing through the highlights.
    let grain = fbm(u * c.aspect * 120.0, v * 120.0, 3, 5);
    let paper = p.n("paper");
    let tex = 1.0 - paper * 0.25 * (grain - 0.5) * 2.0;
    let lit = smoothstep(0.75, 1.0, luma(pooled)) * paper;
    let out = mix(scale(pooled, tex), p.c("paperColor"), lit);
    mix(c.sample(u, v), out, p.n("strength"))
}

pub fn pixelate(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let cell = c.w / p.n("cells").max(1.0); // square cells in pixels
    let (gx, gy) = (u * c.w / cell, v * c.h / cell);
    let (cx, cy) = (gx.floor() + 0.5, gy.floor() + 0.5);
    let col = c.sample(cx * cell / c.w, cy * cell / c.h);
    let gap = p.n("gap");
    let half = 0.5 * (1.0 - gap);
    if gap <= 0.0 && p.n("roundness") <= 0.0 {
        return col;
    }
    let rad = p.n("roundness") * half;
    let (dx, dy) = (((gx - cx).abs() - half + rad).max(0.0), ((gy - cy).abs() - half + rad).max(0.0));
    let outside = (dx * dx + dy * dy).sqrt() - rad + ((gx - cx).abs() - half).max((gy - cy).abs() - half).min(0.0);
    let aa = 1.0 / cell;
    scale(col, 1.0 - smoothstep(-aa, aa, outside))
}

pub fn chromatic_aberration(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (dx, dy) = rot(1.0, 0.0, p.n("angle"));
    // Stronger towards the frame edges, as with a real lens.
    let r = (((u - 0.5) * c.aspect).powi(2) + (v - 0.5).powi(2)).sqrt();
    let k = p.n("strength") * 0.02 * (0.35 + r * 1.3);
    let at = |o: f32| (u + dx * o * k / c.aspect, v + dy * o * k);
    let (r0, g0, b0) = (at(p.n("redOffset")), at(p.n("greenOffset")), at(p.n("blueOffset")));
    [c.sample(r0.0, r0.1)[0], c.sample(g0.0, g0.1)[1], c.sample(b0.0, b0.1)[2]]
}

pub fn glitch(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let i = p.n("intensity");
    let seed = (p.n("seed") * 1000.0) as u32;
    let tick = (c.t * p.n("speed") * 12.0).floor() as i32;
    let dens = p.n("blockDensity");
    let (bx, by) = ((u * dens * 0.5).floor() as i32, (v * dens).floor() as i32);
    let mut uu = u;
    let row = hash21(by, tick, seed);
    if row < i * 0.45 {
        uu += (hash21(bx, by + tick * 7, seed + 1) - 0.5) * 0.25 * i;
    }
    if hash21(bx, by + tick * 3, seed + 2) < p.n("mirrorAmount") * i * 0.3 {
        uu = 1.0 - uu;
    }
    let shift = p.n("rgbShift") / c.w * (0.4 + i) * if row < i { 2.0 } else { 1.0 };
    let mut o = [c.sample(uu + shift, v)[0], c.sample(uu, v)[1], c.sample(uu - shift, v)[2]];
    let bar = hash21(0, (v * 40.0).floor() as i32 + tick * 13, seed + 3);
    if bar < p.n("colorBarIntensity") * i * 0.25 {
        let tint = [hash21(1, tick, seed), hash21(2, tick, seed), hash21(3, tick, seed)];
        o = mix(o, tint, 0.6);
    }
    let line = if (v * c.h) as i32 % 2 == 0 { 1.0 - p.n("scanlineIntensity") * 0.5 } else { 1.0 };
    scale(o, line)
}

pub fn vhs(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed");
    let wob = (noise2(v * 40.0, t * 3.0, 21) - 0.5) * p.n("wobble") * 0.006
        + (v * 3.0 + t * 1.7).sin() * p.n("wobble") * 0.0015;
    // A tracking band that rolls down the frame.
    let band_y = fract(t * 0.13);
    let band = smoothstep(0.035, 0.0, (v - band_y).abs()) * p.n("scanlineNoise");
    let uu = u + wob + band * (noise2(v * 200.0, t * 30.0, 3) - 0.5) * 0.04;
    let y = luma(c.sample(uu, v));
    // Chroma is band-limited on tape: smear it horizontally.
    let sm = p.n("smear") * 0.006;
    let ch = c.sample(uu - sm, v);
    let cy = luma(ch);
    let mut o = [y + (ch[0] - cy) * 0.85, y + (ch[1] - cy) * 0.85, y + (ch[2] - cy) * 0.85];
    let grain = (hash21((u * c.w) as i32, (v * c.h) as i32, (t * 60.0) as u32) - 0.5) * 0.12 * p.n("scanlineNoise");
    let lines = 1.0 - 0.08 * ((v * c.h * PI * 0.5).sin().powi(2)) * p.n("scanlineNoise");
    for k in &mut o {
        *k = (*k + grain + band * 0.25) * lines;
    }
    o
}

pub fn crt_screen(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    // Slight barrel curvature of the tube.
    let (x, y) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
    let k = 0.06;
    let (x, y) = (x * (1.0 + k * y * y), y * (1.0 + k * x * x));
    let (cu, cv) = (x * 0.5 + 0.5, y * 0.5 + 0.5);
    if !(0.0..=1.0).contains(&cu) || !(0.0..=1.0).contains(&cv) {
        return [0.0; 3];
    }
    let rows = p.n("pixelSize");
    let cell = c.h / rows;
    let (gx, gy) = (cu * c.w / cell, cv * c.h / cell);
    let sh = p.n("colorShift") / c.w;
    let at = |o: f32| c.sample(((gx.floor() + 0.5) * cell / c.w + o).clamp(0.0, 1.0), (gy.floor() + 0.5) * cell / c.h);
    let col = [at(-sh)[0], at(0.0)[1], at(sh)[2]];
    // Phosphor triads: each cell is three vertical stripes.
    // Triads need a few device pixels per cell; below that they only alias, so fade them out.
    let sub = (fract(gx) * 3.0).floor() as usize;
    let lo = 1.0 - 0.65 * smoothstep(2.5, 6.0, cell);
    let mut mask = [lo; 3];
    mask[sub.min(2)] = 1.0;
    let scan = 1.0 - p.n("scanlineIntensity") * (cv * p.n("scanlineFrequency") * PI).sin().powi(2);
    let r = ((cu - 0.5).powi(2) + (cv - 0.5).powi(2)).sqrt();
    let vr = p.n("vignetteRadius").max(0.01);
    let vig = 1.0 - p.n("vignetteIntensity") * smoothstep(vr * 0.6, vr * 0.6 + 0.45, r);
    let gain = p.n("brightness") * scan * vig * 1.35;
    [col[0] * mask[0] * gain, col[1] * mask[1] * gain, col[2] * mask[2] * gain]
}

fn screen_blend(a: V3, b: V3) -> V3 {
    [1.0 - (1.0 - a[0]) * (1.0 - b[0]), 1.0 - (1.0 - a[1]) * (1.0 - b[1]), 1.0 - (1.0 - a[2]) * (1.0 - b[2])]
}

pub fn light_leak(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed");
    let pos = p.pt("position");
    let drift = (noise2(t * 0.3, 0.5, 7) - 0.5) * 0.2;
    let (x, y) = c.rel(u, v, (pos.0 + drift, pos.1 - drift * 0.5));
    let r = (x * x + y * y).sqrt() / p.n("spread");
    let a = y.atan2(x);
    let streak = 1.0 + p.n("streaks") * (fbm(a * 3.0 + t * 0.2, t * 0.1, 3, 9) - 0.5) * 1.6;
    let glow = (-r * r * 2.2).exp() * streak.max(0.0);
    let col = if r < 0.5 {
        mix(p.c("hotColor"), p.c("midColor"), r * 2.0)
    } else {
        mix(p.c("midColor"), p.c("fringeColor"), ((r - 0.5) * 1.5).min(1.0))
    };
    let k = (glow * p.n("intensity") * 4.0).min(1.0);
    screen_blend(c.sample(u, v), scale(col, k))
}

pub fn lens_flare(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed");
    let l = p.pt("position");
    let flick = 1.0 + 0.08 * (t * 7.0).sin() * (t * 3.1).cos();
    let (x, y) = c.rel(u, v, l);
    let d = (x * x + y * y).sqrt();
    let mut add_c = [0.0f32; 3];
    // Core glow and starburst around the light.
    let a = y.atan2(x);
    let star = ((a * 6.0 + t * 0.3).cos().abs().powf(40.0) + (a * 9.0 - t * 0.2).cos().abs().powf(60.0) * 0.6)
        * (-d * 5.0).exp();
    let core = (-d * 16.0).exp() + star * p.n("starburst") * 1.5;
    add_c = add(add_c, scale([1.0, 0.95, 0.85], core));
    // Anamorphic streak along the horizontal.
    let streak = (-y.abs() * 220.0).exp() * (-x.abs() * 1.4).exp() * p.n("streak") * 1.5;
    add_c = add(add_c, scale([0.6, 0.75, 1.0], streak));
    // Ghosts on the axis through the frame centre.
    let (ax, ay) = ((0.5 - l.0) * c.aspect, 0.5 - l.1);
    let tints: [V3; 5] = [[0.4, 0.8, 1.0], [1.0, 0.6, 0.3], [0.5, 1.0, 0.6], [0.9, 0.4, 1.0], [1.0, 0.9, 0.5]];
    for (i, k) in [0.6f32, 1.2, 1.55, 2.1, 2.6].iter().enumerate() {
        let (gx, gy) = (x - ax * k, y - ay * k);
        let gd = (gx * gx + gy * gy).sqrt();
        let rad = 0.03 + 0.025 * i as f32;
        let disc = smoothstep(rad, rad * 0.7, gd) * 0.25;
        add_c = add(add_c, scale(tints[i], disc * p.n("ghosts")));
    }
    // Halo ring around the frame centre.
    let (hx, hy) = (x - ax, y - ay);
    let hd = (hx * hx + hy * hy).sqrt();
    let ring = (-((hd - 0.35) * 30.0).powi(2)).exp() * 0.35 * p.n("halo");
    add_c = add(add_c, scale([1.0, 0.7, 0.5], ring));
    let s = c.sample(u, v);
    let k = p.n("intensity") * flick;
    [s[0] + add_c[0] * k, s[1] + add_c[1] * k, s[2] + add_c[2] * k]
}

pub fn zoom_blur(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (cx, cy) = p.pt("center");
    let s = p.n("intensity") / 100.0 * 0.35;
    let n = 20;
    let j = hash21((u * c.w) as i32, (v * c.h) as i32, 4) / n as f32;
    let mut acc = [0.0; 3];
    for i in 0..n {
        let k = 1.0 - s * (i as f32 / n as f32 + j);
        acc = add(acc, c.sample(cx + (u - cx) * k, cy + (v - cy) * k));
    }
    scale(acc, 1.0 / n as f32)
}

/// Disc blur of radius `r` (normalised to frame height) with golden-angle taps.
pub fn disc_blur(c: &Ctx, u: f32, v: f32, r: f32, taps: usize) -> V3 {
    if r * c.h < 0.5 {
        return c.sample(u, v);
    }
    let mut acc = [0.0; 3];
    let j = hash21((u * c.w) as i32, (v * c.h) as i32, 8) * TAU;
    for i in 0..taps {
        let a = j + i as f32 * 2.399_963;
        let rr = r * ((i as f32 + 0.5) / taps as f32).sqrt();
        acc = add(acc, c.sample(u + a.cos() * rr / c.aspect, v + a.sin() * rr));
    }
    scale(acc, 1.0 / taps as f32)
}

pub fn tilt_shift(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = c.rel(u, v, p.pt("center"));
    let (_, d) = rot(x, y, -p.n("angle"));
    let half = p.n("width") * 0.5;
    let k = smoothstep(half, half + p.n("falloff").max(0.01), d.abs());
    let r = k * p.n("intensity") / 100.0 * 0.02;
    let o = disc_blur(c, u, v, r, 16);
    // A touch of saturation sells the miniature look.
    let l = luma(o);
    mix([l; 3], o, 1.0 + 0.15 * (1.0 - k))
}
