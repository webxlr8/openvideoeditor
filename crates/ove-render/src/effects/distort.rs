//! Distort effects: each remaps where a pixel samples the source from.

use std::f32::consts::{PI, TAU};

use super::util::*;

pub fn bulge(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ctr = p.pt("center");
    let (x, y) = c.rel(u, v, ctr);
    let r = (x * x + y * y).sqrt();
    let rad = p.n("radius") * 0.5;
    if r >= rad {
        return c.sample(u, v);
    }
    let f = 1.0 - r / rad;
    let shape = f.powf(1.0 + p.n("falloff") * 2.0);
    let k = (1.0 - p.n("strength") * shape * 0.85).max(0.05);
    let (su, sv) = c.abs(x * k, y * k, ctr);
    c.sample(su, sv)
}

pub fn spherize(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ctr = p.pt("center");
    let (x, y) = c.rel(u, v, ctr);
    let rad = p.n("radius") * 0.5;
    let r = (x * x + y * y).sqrt();
    if r >= rad {
        return c.sample(u, v);
    }
    let q = r / rad;
    // Map the disc onto a hemisphere: the surface angle spreads the centre and compresses the rim.
    let sph = q.asin() / (PI * 0.5);
    let depth = p.n("depth");
    let nq = (q + (sph - q) * depth).max(0.0);
    let k = if q > 1e-5 { nq / q } else { 1.0 - depth * 0.36 };
    let (su, sv) = c.abs(x * k, y * k, ctr);
    let base = c.sample(su, sv);
    // Light the sphere from the light position (relative to the centre).
    let z = (1.0 - q * q).max(0.0).sqrt();
    let (n0, n1, n2) = (x / rad, y / rad, z);
    let lp = p.pt("lightPosition");
    let (lx, ly) = ((lp.0 - 0.5) * 2.0, (lp.1 - 0.5) * 2.0);
    let ll = (lx * lx + ly * ly + 0.64).sqrt();
    let ndl = ((n0 * lx + n1 * ly + n2 * 0.8) / ll).max(0.0);
    let shin = 4.0 + (1.0 - p.n("lightSoftness")) * 60.0;
    let spec = ndl.powf(shin) * p.n("lightIntensity");
    let shade = 1.0 - p.n("lightIntensity") * 0.35 * (1.0 - ndl);
    let lc = p.c("lightColor");
    [base[0] * shade + lc[0] * spec, base[1] * shade + lc[1] * spec, base[2] * shade + lc[2] * spec]
}

pub fn lens_distortion(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ctr = p.pt("center");
    let (x, y) = c.rel(u, v, ctr);
    let (x, y) = rot(x, y, -p.n("angle"));
    let round_r = (x * x + y * y).sqrt();
    let square_r = x.abs().max(y.abs());
    let r = round_r + (square_r - round_r) * p.n("roundness");
    let spread = p.n("spread").max(0.05);
    let clear = p.n("clearCenter") * spread * 0.5;
    let w = smoothstep(clear, spread, r);
    let k = 1.0 - p.n("bulge") * w * 0.5;
    let sw = p.n("swirl") * w * PI * 0.25;
    let (x, y) = rot(x * k, y * k, sw.to_degrees());
    let (x, y) = rot(x, y, p.n("angle"));
    let (su, sv) = c.abs(x, y, ctr);
    c.sample(su, sv)
}

pub fn twirl(c: &Ctx, u: f32, v: f32) -> V3 {
    let ctr = c.p.pt("center");
    let (x, y) = c.rel(u, v, ctr);
    let r = (x * x + y * y).sqrt();
    let f = (1.0 - r / 0.6).max(0.0);
    let (x, y) = rot(x, y, (c.p.n("intensity") * f * f * PI).to_degrees());
    let (su, sv) = c.abs(x, y, ctr);
    c.sample(su, sv)
}

pub fn wave_distortion(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = rot((u - 0.5) * c.aspect, v - 0.5, -p.n("angle"));
    let off = (y * p.n("frequency") * TAU * 3.0 + c.t * p.n("speed") * TAU).sin() * p.n("strength") * 0.06;
    let (x, y) = rot(x + off, y, p.n("angle"));
    c.sample(x / c.aspect + 0.5, y + 0.5)
}

pub fn kaleidoscope(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ctr = p.pt("center");
    let (x, y) = c.rel(u, v, ctr);
    let r = (x * x + y * y).sqrt();
    let seg = TAU / p.n("segments").round().max(2.0);
    let a0 = p.n("angle").to_radians();
    let mut a = (y.atan2(x) - a0).rem_euclid(seg);
    if a > seg * 0.5 {
        a = seg - a;
    }
    let a = a + a0;
    let (su, sv) = c.abs(r * a.cos(), r * a.sin(), ctr);
    // Fold back into the frame instead of clamping, so edges stay patterned.
    let fold = |t: f32| 1.0 - (fract(t * 0.5) * 2.0 - 1.0).abs();
    c.sample(fold(su), fold(sv))
}

pub fn mirror(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ctr = p.pt("center");
    let (x, y) = c.rel(u, v, ctr);
    // Mirror line through the centre with direction `angle`; reflect the negative side onto the positive.
    let (nx, ny) = rot(0.0, 1.0, p.n("angle"));
    let d = x * nx + y * ny;
    let (x, y) = if d < 0.0 { (x - 2.0 * d * nx, y - 2.0 * d * ny) } else { (x, y) };
    let (su, sv) = c.abs(x, y, ctr);
    c.sample(su, sv)
}

pub fn polar_coordinates(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let ctr = p.pt("center");
    // The output's x is the angle and y the radius: the source is unrolled around the centre.
    let a = (u - 0.5) * TAU * p.n("wrap");
    let r = v * p.n("radius") * 0.5;
    let (pu, pv) = c.abs(r * a.sin(), -r * a.cos(), ctr);
    let k = p.n("intensity");
    c.sample(u + (pu - u) * k, v + (pv - v) * k)
}

pub fn fluted_glass(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = rot((u - 0.5) * c.aspect, v - 0.5, -p.n("angle"));
    let f = p.n("frequency");
    let s = x * f + c.t * p.n("speed");
    let local = fract(s) - 0.5; // -0.5..0.5 across one flute
    let soft = 0.5 + p.n("softness") * 1.5;
    let profile = (local * PI).sin() * (1.0 - local.abs() * 2.0).max(0.0).powf(1.0 / soft);
    let off = -profile * p.n("refraction") * 0.08 / f.max(1.0);
    let ab = p.n("aberration") * 0.25;
    let at = |k: f32| {
        let (rx, ry) = rot(x + off * k, y, p.n("angle"));
        c.sample(rx / c.aspect + 0.5, ry + 0.5)
    };
    let hl = (1.0 - (local * 2.0 - 0.25).abs() * 4.0).max(0.0) * p.n("highlight") * 0.5;
    let col = [at(1.0 - ab)[0], at(1.0)[1], at(1.0 + ab)[2]];
    [col[0] + hl, col[1] + hl, col[2] + hl]
}

pub fn glass_tiles(c: &Ctx, u: f32, v: f32) -> V3 {
    let p = c.p;
    let (x, y) = rot((u - 0.5) * c.aspect, v - 0.5, -p.n("rotation"));
    let n = p.n("tileCount");
    let (gx, gy) = (x * n, y * n);
    let (lx, ly) = (fract(gx) - 0.5, fract(gy) - 0.5);
    // Each tile is a small lens: content is shifted outward from the tile centre.
    let k = p.n("intensity") * 0.02;
    let (sx, sy) = rot(x + lx * k, y + ly * k, p.n("rotation"));
    let col = c.sample(sx / c.aspect + 0.5, sy + 0.5);
    let round = p.n("roundness");
    let edge = if round > 0.0 {
        let r = 0.5 * round;
        let (dx, dy) = ((lx.abs() - 0.5 + r).max(0.0), (ly.abs() - 0.5 + r).max(0.0));
        smoothstep(r - 0.04, r + 0.02, (dx * dx + dy * dy).sqrt())
    } else {
        0.0
    };
    let bevel = 1.0 - 0.18 * smoothstep(0.38, 0.5, lx.abs().max(ly.abs())) - 0.7 * edge;
    scale(col, bevel)
}
