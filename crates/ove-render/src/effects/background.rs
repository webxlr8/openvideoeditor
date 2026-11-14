//! Background generators: procedural images that ignore the source.

use std::f32::consts::{PI, TAU};

use super::util::*;

/// Aspect-corrected coordinates centred on the frame, height spanning -0.5..0.5.
fn centred(c: &Ctx, u: f32, v: f32) -> (f32, f32) {
    ((u - 0.5) * c.aspect, v - 0.5)
}

pub fn aurora(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.03;
    let (x, _) = centred(c, u, v);
    let mut col = p.c("sky");
    // Faint stars.
    let star = hash21((u * c.w / 2.0) as i32, (v * c.h / 2.0) as i32, 77);
    if star > 0.997 {
        col = add(col, [0.5 * (1.0 - v); 3]);
    }
    let cols = [p.c("colorA"), p.c("colorB"), p.c("colorC")];
    for (i, cc) in cols.iter().enumerate() {
        let fi = i as f32;
        let curtain = 0.32
            + fi * 0.09
            + (x * 2.3 + t * (1.0 + fi * 0.3) + fi * 2.0).sin() * 0.08 * p.n("waviness")
            + (fbm(x * 1.5 + fi * 3.0, t * 0.7, 3, 31 + i as u32) - 0.5) * 0.25 * p.n("waviness");
        let h = 0.12 * p.n("height");
        let dy = curtain - v; // positive above the curtain's lower edge
        let body = if dy > 0.0 { (-dy / h).exp() } else { (-(dy / 0.015).powi(2)).exp() };
        let rays = 0.55 + 0.45 * fbm(x * 18.0 + t * 2.0 + fi * 5.0, fi, 3, 41 + i as u32);
        col = add(col, scale(*cc, body * rays * p.n("intensity") * 0.75));
    }
    col
}

pub fn mesh_gradient(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.15;
    let (mut x, mut y) = centred(c, u, v);
    let r = (x * x + y * y).sqrt();
    (x, y) = rot(x, y, p.n("swirl") * (1.0 - r * 1.5).max(0.0) * 180.0);
    let cols = [p.c("color1"), p.c("color2"), p.c("color3"), p.c("color4"), p.c("color5")];
    let pw = 1.0 + p.n("smoothness") * 0.8;
    let (mut acc, mut wsum) = ([0.0; 3], 0.0);
    for (i, cc) in cols.iter().enumerate() {
        let fi = i as f32;
        let px = (t * (0.7 + fi * 0.13) + fi * 1.7).sin() * 0.45 * c.aspect;
        let py = (t * (0.5 + fi * 0.11) + fi * 2.9).cos() * 0.38;
        let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt().max(1e-3);
        let w = 1.0 / d.powf(pw * 1.6);
        acc = add(acc, scale(*cc, w));
        wsum += w;
    }
    scale(acc, 1.0 / wsum)
}

pub fn flowing_gradient(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.06;
    let seed = p.n("seed") as u32;
    let (x, y) = centred(c, u, v);
    let d = p.n("distortion");
    let wx = fbm(x * 1.4 + t, y * 1.4, 4, seed) - 0.5;
    let wy = fbm(x * 1.4, y * 1.4 - t, 4, seed + 9) - 0.5;
    let k = fbm(x * 1.1 + wx * d * 3.0, y * 1.1 + wy * d * 3.0 + t * 0.5, 4, seed + 17);
    let s = smoothstep(0.2, 0.8, k * 0.7 + (v + x * 0.2) * 0.3);
    ramp(&[p.c("colorA"), p.c("colorB"), p.c("colorC"), p.c("colorD")], s)
}

pub fn plasma(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.5;
    let (x, y) = centred(c, u, v);
    let den = p.n("density") * 3.0 + 1.0;
    let warp = p.n("warp") * 2.0 * (fbm(x * 2.0, y * 2.0 + t * 0.2, 3, 51) - 0.5);
    let (x, y) = (x + warp, y - warp);
    let s = (x * den + t).sin()
        + (y * den * 1.3 - t * 1.2).sin()
        + ((x + y) * den * 0.7 + t * 0.8).sin()
        + ((x * x + y * y).sqrt() * den * 2.0 - t * 1.5).sin();
    let k = (s * 0.125 + 0.5).clamp(0.0, 1.0);
    let k = (k + p.n("balance") - 0.5).clamp(0.0, 1.0).powf(1.0 / p.n("intensity"));
    mix(p.c("colorB"), p.c("colorA"), k)
}

pub fn swirl(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.1;
    let (x, y) = centred(c, u, v);
    let r = (x * x + y * y).sqrt();
    let (x, y) = rot(x, y, ((1.0 - r).max(0.0) * 3.0 + t).to_degrees());
    let oct = 1 + p.n("detail").round() as u32;
    let n = fbm(x * 3.0 + t, y * 3.0, oct, 61);
    let b = p.n("blend");
    mix(p.c("colorA"), p.c("colorB"), smoothstep(0.5 - b * 0.5, 0.5 + b * 0.5, n))
}

pub fn marble(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed");
    let seed = p.n("seed") as u32;
    let (x, y) = centred(c, u, v);
    let s = p.n("scale") * 4.0;
    let turb = fbm(x * s + t, y * s, 5, seed) * p.n("turbulence") * 0.25;
    let vein = ((x + y * 0.6) * s * 2.0 + turb).sin() * 0.5 + 0.5;
    let k = vein.powf(0.6);
    ramp(&[p.c("colorC"), p.c("colorB"), p.c("colorA")], k)
}

pub fn godrays(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.3;
    let (x, y) = c.rel(u, v, p.pt("center"));
    let r = (x * x + y * y).sqrt();
    let a = y.atan2(x);
    let den = 4.0 + p.n("density") * 40.0;
    let rays = noise2(a * den, t, 71) * 0.6 + noise2(a * den * 2.3, -t * 1.3, 72) * 0.4;
    let rays = smoothstep(0.35, 0.9, rays);
    let spots = 1.0 - p.n("spotty") * 0.7 * (1.0 - fbm(r * 6.0 - t * 2.0, a * 3.0, 3, 73));
    let fall = (-r * 1.2).exp();
    let k = (rays * spots * fall * p.n("intensity") * 1.6).clamp(0.0, 1.0);
    mix(p.c("background"), p.c("rayColor"), k)
}

pub fn sunburst(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = c.rel(u, v, p.pt("center"));
    let r = (x * x + y * y).sqrt();
    let a = y.atan2(x) + c.t * p.n("speed");
    let n = p.n("rayCount").round();
    let w = (a * n * 0.5).cos().abs(); // 1 on a ray, 0 between rays
    let soft = p.n("softness").max(0.01) * 0.5;
    let ray = smoothstep(0.5 - soft, 0.5 + soft, w);
    let rad = p.n("radius").max(0.01);
    let mask = 1.0 - smoothstep(rad * (1.0 - p.n("feather")), rad, r);
    mix(p.c("background"), p.c("color"), ray * mask)
}

pub fn spiral(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = c.rel(u, v, p.pt("center"));
    let r = (x * x + y * y).sqrt();
    let a = y.atan2(x) / TAU;
    let s = fract(r * p.n("scale") * 2.0 + a - c.t * p.n("speed") * 0.25);
    let d = (s - 0.5).abs() * 2.0; // 0 at the stroke centre
    let width = p.n("strokeWidth") * (1.0 - p.n("strokeFalloff") * (r * 1.5).min(1.0));
    let soft = p.n("softness").max(0.005);
    let stroke = 1.0 - smoothstep(width - soft, width + soft, d);
    mix(p.c("colorA"), p.c("colorB"), stroke)
}

pub fn conic_gradient(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = c.rel(u, v, p.pt("center"));
    let a = (y.atan2(x) + PI) / TAU + p.n("rotation") / 360.0;
    let k = 1.0 - (fract(a * p.n("repeat").round()) * 2.0 - 1.0).abs();
    mix(p.c("colorA"), p.c("colorB"), k)
}

pub fn voronoi(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed");
    let (x, y) = centred(c, u, v);
    let s = p.n("scale");
    let (gx, gy) = (x * s, y * s);
    let (ix, iy) = (gx.floor() as i32, gy.floor() as i32);
    let (mut d1, mut d2, mut id) = (9.0f32, 9.0f32, (0, 0));
    for oy in -1..=1 {
        for ox in -1..=1 {
            let (cx, cy) = (ix + ox, iy + oy);
            let h1 = hash21(cx, cy, 81);
            let h2 = hash21(cx, cy, 82);
            let px = cx as f32 + 0.5 + 0.4 * (t + h1 * TAU).sin();
            let py = cy as f32 + 0.5 + 0.4 * (t * 0.8 + h2 * TAU).cos();
            let d = ((gx - px).powi(2) + (gy - py).powi(2)).sqrt();
            if d < d1 {
                d2 = d1;
                d1 = d;
                id = (cx, cy);
            } else if d < d2 {
                d2 = d;
            }
        }
    }
    let fill = mix(p.c("colorA"), p.c("colorB"), hash21(id.0, id.1, 83));
    let edge = 1.0 - smoothstep(0.0, p.n("edgeSoftness") + 0.02, d2 - d1);
    mix(fill, p.c("borderColor"), edge * p.n("edgeIntensity"))
}

pub fn stripes(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = centred(c, u, v);
    let (x, _) = rot(x, y, -p.n("angle"));
    let s = fract(x * p.n("density") + c.t * p.n("speed"));
    let (b, soft) = (p.n("balance"), p.n("softness").max(0.002));
    // Two soft edges per period: rising at 0, falling at the balance point.
    let k = smoothstep(-soft, soft, s.min(1.0 - s)) * (1.0 - smoothstep(b - soft, b + soft, s));
    mix(p.c("colorA"), p.c("colorB"), k)
}

pub fn studio_background(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let t = c.t * p.n("speed") * 0.05;
    let ctr = p.pt("center");
    let ctr = (ctr.0 + t.sin() * 0.05, ctr.1 + (t * 0.7).cos() * 0.02);
    let (x, y) = c.rel(u, v, ctr);
    let r2 = x * x + y * y;
    let size = 0.15 + p.n("softness") * 0.6;
    let light = (-r2 / (size * size)).exp() * p.n("intensity");
    let base = p.c("color");
    // A soft floor gradient: slightly darker towards the bottom.
    let floor = 1.0 - 0.18 * smoothstep(0.4, 1.0, v);
    let lit = mix(scale(base, floor), p.c("lightColor"), light.min(1.0));
    let (vx, vy) = centred(c, u, v);
    let vig = 1.0 - p.n("vignette") * smoothstep(0.3, 0.9, (vx * vx / (c.aspect * c.aspect) + vy * vy).sqrt() * 1.4);
    let grain = (hash21((u * c.w) as i32, (v * c.h) as i32, 91) - 0.5) * 0.01;
    scale(lit, vig).map(|k| k + grain)
}
