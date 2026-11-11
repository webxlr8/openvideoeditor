//! CPU compositing on RGBA8 (straight alpha) images: affine layer drawing with bilinear sampling, crop, rounded
//! corners, opacity and blend modes, and colour grading. Rows are processed in parallel.

use rayon::prelude::*;

use crate::eval::{Blend, Grade};

/// An RGBA8 image, straight alpha, tightly packed.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Rgba {
    pub fn new(w: u32, h: u32) -> Self {
        Rgba { w, h, data: vec![0; (w * h * 4) as usize] }
    }

    pub fn filled(w: u32, h: u32, c: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            data.extend_from_slice(&c);
        }
        Rgba { w, h, data }
    }

    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.w + x) * 4) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    pub fn clear(&mut self, c: [u8; 4]) {
        for p in self.data.chunks_exact_mut(4) {
            p.copy_from_slice(&c);
        }
    }
}

/// Where a source image goes: centre in destination pixels, destination pixels per source pixel (negative `sx`
/// mirrors), rotation in degrees (clockwise).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub cx: f64,
    pub cy: f64,
    pub sx: f64,
    pub sy: f64,
    pub rotation: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawParams {
    pub opacity: f32,
    pub blend: Blend,
    /// top, right, bottom, left in percent of the source.
    pub crop: [f32; 4],
    /// Corner radius in source pixels.
    pub radius: f32,
}

impl Default for DrawParams {
    fn default() -> Self {
        DrawParams { opacity: 1.0, blend: Blend::Normal, crop: [0.0; 4], radius: 0.0 }
    }
}

#[inline]
fn blend_ch(mode: Blend, d: f32, s: f32) -> f32 {
    match mode {
        Blend::Normal => s,
        Blend::Multiply => d * s,
        Blend::Screen => 1.0 - (1.0 - d) * (1.0 - s),
        Blend::Darken => d.min(s),
        Blend::Lighten => d.max(s),
        Blend::Add => (d + s).min(1.0),
    }
}

/// Composites `s` (straight RGB in 0..1, alpha `a` already including opacity and coverage) over destination pixel.
#[inline]
fn put(dst: &mut [u8], s: [f32; 3], a: f32, mode: Blend) {
    if a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    for c in 0..3 {
        let d = dst[c] as f32 / 255.0;
        let b = if mode == Blend::Normal { s[c] } else { blend_ch(mode, d, s[c]) };
        // Over a transparent destination the blend mode has nothing to act on: use the source colour.
        let b = b * da + s[c] * (1.0 - da);
        dst[c] = ((d * (1.0 - a) + b * a) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    dst[3] = ((da + a * (1.0 - da)) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
}

#[inline]
fn sample(src: &Rgba, u: f64, v: f64) -> [f32; 4] {
    // Bilinear, clamped to the edge; u, v in source pixel coordinates (pixel centres at +0.5).
    let x = (u - 0.5).clamp(0.0, (src.w - 1) as f64);
    let y = (v - 0.5).clamp(0.0, (src.h - 1) as f64);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(src.w - 1), (y0 + 1).min(src.h - 1));
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let p = |x: u32, y: u32| {
        let i = ((y * src.w + x) * 4) as usize;
        [src.data[i] as f32, src.data[i + 1] as f32, src.data[i + 2] as f32, src.data[i + 3] as f32]
    };
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut o = [0f32; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        o[k] = (top + (bot - top) * fy) / 255.0;
    }
    o
}

/// Draws `src` onto `dst`.
pub fn draw(dst: &mut Rgba, src: &Rgba, place: Place, p: &DrawParams) {
    if src.w == 0 || src.h == 0 || p.opacity <= 0.0 || place.sx == 0.0 || place.sy == 0.0 {
        return;
    }
    let (sw, sh) = (src.w as f64, src.h as f64);
    let crop_l = sw * p.crop[3] as f64 / 100.0;
    let crop_r = sw - sw * p.crop[1] as f64 / 100.0;
    let crop_t = sh * p.crop[0] as f64 / 100.0;
    let crop_b = sh - sh * p.crop[2] as f64 / 100.0;
    if crop_r <= crop_l || crop_b <= crop_t {
        return;
    }
    let no_crop = p.crop == [0.0; 4] && p.radius <= 0.0;

    // Fast path: unscaled, unrotated, unmirrored, pixel-aligned.
    let ox = place.cx - sw / 2.0;
    let oy = place.cy - sh / 2.0;
    if no_crop
        && place.rotation == 0.0
        && (place.sx - 1.0).abs() < 1e-9
        && (place.sy - 1.0).abs() < 1e-9
        && (ox - ox.round()).abs() < 1e-6
        && (oy - oy.round()).abs() < 1e-6
    {
        let (ox, oy) = (ox.round() as i64, oy.round() as i64);
        let dw = dst.w as i64;
        let x0 = ox.max(0);
        let x1 = (ox + src.w as i64).min(dw);
        if x1 <= x0 {
            return;
        }
        let opacity = p.opacity;
        let blend = p.blend;
        dst.data.par_chunks_exact_mut((dst.w * 4) as usize).enumerate().for_each(|(y, row)| {
            let sy = y as i64 - oy;
            if sy < 0 || sy >= src.h as i64 {
                return;
            }
            let srow = &src.data[(sy as usize * src.w as usize * 4)..][..src.w as usize * 4];
            for x in x0..x1 {
                let si = ((x - ox) * 4) as usize;
                let s = &srow[si..si + 4];
                let a = s[3] as f32 / 255.0 * opacity;
                let d = &mut row[(x * 4) as usize..(x * 4 + 4) as usize];
                if a >= 1.0 && blend == Blend::Normal {
                    d.copy_from_slice(s);
                    continue;
                }
                put(d, [s[0] as f32 / 255.0, s[1] as f32 / 255.0, s[2] as f32 / 255.0], a, blend);
            }
        });
        return;
    }

    // General path: inverse-map every destination pixel of the transformed source's bounding box.
    let th = place.rotation.to_radians();
    let (sin, cos) = th.sin_cos();
    let corners = [(crop_l, crop_t), (crop_r, crop_t), (crop_l, crop_b), (crop_r, crop_b)];
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (u, v) in corners {
        let (lx, ly) = ((u - sw / 2.0) * place.sx, (v - sh / 2.0) * place.sy);
        let (x, y) = (lx * cos - ly * sin + place.cx, lx * sin + ly * cos + place.cy);
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let x0 = minx.floor().max(0.0) as u32;
    let y0 = miny.floor().max(0.0) as u32;
    let x1 = (maxx.ceil().max(0.0) as u32).min(dst.w);
    let y1 = (maxy.ceil().max(0.0) as u32).min(dst.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let radius = p.radius as f64;
    let (opacity, blend) = (p.opacity, p.blend);
    let dw = dst.w as usize;
    dst.data.par_chunks_exact_mut(dw * 4).enumerate().skip(y0 as usize).take((y1 - y0) as usize).for_each(
        |(y, row)| {
            let dy = y as f64 + 0.5 - place.cy;
            for x in x0..x1 {
                let dx = x as f64 + 0.5 - place.cx;
                // Inverse rotation, then inverse scale, into source coordinates.
                let rx = dx * cos + dy * sin;
                let ry = -dx * sin + dy * cos;
                let u = rx / place.sx + sw / 2.0;
                let v = ry / place.sy + sh / 2.0;
                if u < crop_l || u >= crop_r || v < crop_t || v >= crop_b {
                    continue;
                }
                let mut cover = 1.0f32;
                if radius > 0.0 {
                    let cxr = if u < crop_l + radius {
                        crop_l + radius
                    } else if u > crop_r - radius {
                        crop_r - radius
                    } else {
                        u
                    };
                    let cyr = if v < crop_t + radius {
                        crop_t + radius
                    } else if v > crop_b - radius {
                        crop_b - radius
                    } else {
                        v
                    };
                    let dist = ((u - cxr).powi(2) + (v - cyr).powi(2)).sqrt();
                    cover = (radius - dist + 0.5).clamp(0.0, 1.0) as f32;
                    if cover <= 0.0 {
                        continue;
                    }
                }
                let s = sample(src, u, v);
                let i = x as usize * 4;
                put(&mut row[i..i + 4], [s[0], s[1], s[2]], s[3] * opacity * cover, blend);
            }
        },
    );
}

/// Applies a colour grade in place (all pixels).
pub fn grade(img: &mut Rgba, g: &Grade) {
    // Per-channel tone curve as a lookup table: exposure, brightness, contrast, gamma, shadows/highlights.
    let mut lut = [[0u8; 256]; 3];
    let gain = 2f32.powf(g.exposure);
    let warm = [1.0 + 0.12 * g.temperature, 1.0 - 0.10 * g.tint, 1.0 - 0.12 * g.temperature];
    let gamma = 1.0 / (1.0 + g.gamma).max(0.1);
    for (c, table) in lut.iter_mut().enumerate() {
        for (i, out) in table.iter_mut().enumerate() {
            let mut v = i as f32 / 255.0;
            v *= gain * warm[c];
            v += g.brightness * 0.5;
            v = (v - 0.5) * (1.0 + g.contrast) + 0.5;
            v += g.shadows * 0.25 * (1.0 - v).max(0.0).powi(2) + g.highlights * 0.25 * v.max(0.0).powi(2);
            v = v.clamp(0.0, 1.0).powf(gamma);
            *out = (v * 255.0 + 0.5) as u8;
        }
    }
    let sat = 1.0 + g.saturation;
    let vib = g.vibrance;
    img.data.par_chunks_exact_mut(4).for_each(|p| {
        let mut rgb = [lut[0][p[0] as usize] as f32, lut[1][p[1] as usize] as f32, lut[2][p[2] as usize] as f32];
        if sat != 1.0 || vib != 0.0 {
            let l = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            let mx = rgb[0].max(rgb[1]).max(rgb[2]);
            let mn = rgb[0].min(rgb[1]).min(rgb[2]);
            let cur = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
            let s = sat * (1.0 + vib * (1.0 - cur));
            for v in rgb.iter_mut() {
                *v = l + (*v - l) * s;
            }
        }
        for c in 0..3 {
            p[c] = (rgb[c] + 0.5).clamp(0.0, 255.0) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_path_copies_and_blends() {
        let mut dst = Rgba::filled(8, 8, [0, 0, 0, 255]);
        let src = Rgba::filled(4, 4, [200, 100, 50, 255]);
        draw(&mut dst, &src, Place { cx: 4.0, cy: 4.0, sx: 1.0, sy: 1.0, rotation: 0.0 }, &DrawParams::default());
        assert_eq!(dst.px(2, 2), [200, 100, 50, 255]);
        assert_eq!(dst.px(1, 1), [0, 0, 0, 255]);
        let half = DrawParams { opacity: 0.5, ..Default::default() };
        let mut dst = Rgba::filled(8, 8, [0, 0, 0, 255]);
        draw(&mut dst, &src, Place { cx: 4.0, cy: 4.0, sx: 1.0, sy: 1.0, rotation: 0.0 }, &half);
        assert_eq!(dst.px(3, 3), [100, 50, 25, 255]);
    }

    #[test]
    fn scaled_rotated_mirrored_and_cropped() {
        // Left half red, right half blue; mirrored → left blue.
        let mut src = Rgba::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                let i = ((y * 4 + x) * 4) as usize;
                src.data[i..i + 4].copy_from_slice(if x < 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let mut dst = Rgba::filled(16, 16, [0, 0, 0, 255]);
        draw(&mut dst, &src, Place { cx: 8.0, cy: 8.0, sx: -2.0, sy: 2.0, rotation: 0.0 }, &DrawParams::default());
        assert_eq!(dst.px(1, 8), [0, 0, 0, 255]);
        assert_eq!(dst.px(5, 8)[2], 255, "left side is blue when mirrored");
        assert_eq!(dst.px(10, 8)[0], 255);
        // 90° rotation turns the strip vertical.
        let mut dst = Rgba::filled(16, 16, [0, 0, 0, 255]);
        draw(&mut dst, &src, Place { cx: 8.0, cy: 8.0, sx: 2.0, sy: 2.0, rotation: 90.0 }, &DrawParams::default());
        assert_eq!(dst.px(8, 5)[0], 255, "red on top after 90° clockwise");
        assert_eq!(dst.px(8, 10)[2], 255);
        assert_eq!(dst.px(2, 8), [0, 0, 0, 255]);
        // Crop the right 50 %: only red remains.
        let mut dst = Rgba::filled(16, 16, [0, 0, 0, 255]);
        let p = DrawParams { crop: [0.0, 50.0, 0.0, 0.0], ..Default::default() };
        draw(&mut dst, &src, Place { cx: 8.0, cy: 8.0, sx: 2.0, sy: 2.0, rotation: 0.0 }, &p);
        assert_eq!(dst.px(9, 8), [0, 0, 0, 255]);
        assert_eq!(dst.px(5, 8)[0], 255);
    }

    #[test]
    fn blend_modes() {
        let src = Rgba::filled(2, 2, [128, 128, 128, 255]);
        let mut dst = Rgba::filled(2, 2, [255, 64, 0, 255]);
        let p = DrawParams { blend: Blend::Multiply, ..Default::default() };
        draw(&mut dst, &src, Place { cx: 1.0, cy: 1.0, sx: 1.0, sy: 1.0, rotation: 0.0 }, &p);
        assert_eq!(dst.px(0, 0), [128, 32, 0, 255]);
        let mut dst = Rgba::filled(2, 2, [255, 64, 0, 255]);
        let p = DrawParams { blend: Blend::Screen, ..Default::default() };
        draw(&mut dst, &src, Place { cx: 1.0, cy: 1.0, sx: 1.0, sy: 1.0, rotation: 0.0 }, &p);
        assert_eq!(dst.px(0, 0), [255, 160, 128, 255]);
    }

    #[test]
    fn grade_changes_pixels_predictably() {
        let mut img = Rgba::filled(2, 2, [100, 150, 200, 255]);
        grade(&mut img, &Grade { saturation: -1.0, ..Default::default() });
        let p = img.px(0, 0);
        assert!(p[0] == p[1] && p[1] == p[2], "fully desaturated is grey: {p:?}");
        let mut img = Rgba::filled(2, 2, [100, 100, 100, 255]);
        grade(&mut img, &Grade { exposure: 1.0, ..Default::default() });
        assert_eq!(img.px(0, 0)[0], 200);
        let mut img = Rgba::filled(2, 2, [100, 100, 100, 255]);
        grade(&mut img, &Grade { temperature: 1.0, ..Default::default() });
        let p = img.px(0, 0);
        assert!(p[0] > 100 && p[2] < 100);
    }
}
