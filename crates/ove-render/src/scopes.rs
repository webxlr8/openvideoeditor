//! Video scopes (design D7): luma waveform, RGB parade and vectorscope, binned from a composited frame into
//! 256 × 256 count grids and shown as u8 density images. Binning uses integer arithmetic on the 8-bit code values,
//! so the GPU compute pass (`ove-gpu`) produces exactly the same counts.
//!
//! - Waveform: column = x · 256 / width, row = 255 − BT.709 luma (code value), so white is at the top.
//! - Parade: three panels of 85 columns (red, green, blue), row = 255 − channel value.
//! - Vectorscope: Cb to the right and Cr up, ±0.5 mapped to the grid edges (BT.709 on code values).
//!
//! Density is logarithmic in the count, scaled so the fullest bin is 255.

use rayon::prelude::*;
use serde::Serialize;

use crate::canvas::Rgba;

/// Bins per side of every scope grid.
pub const SIZE: usize = 256;
/// Columns per channel panel of the parade.
pub const PARADE_PANEL: usize = 85;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Waveform,
    Parade,
    Vectorscope,
}

/// The scope names, for messages.
pub const NAMES: &[&str] = &["waveform", "parade", "vectorscope"];

impl Scope {
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "waveform" => Some(Scope::Waveform),
            "parade" => Some(Scope::Parade),
            "vectorscope" => Some(Scope::Vectorscope),
            _ => None,
        }
    }

    pub const ALL: [Scope; 3] = [Scope::Waveform, Scope::Parade, Scope::Vectorscope];
}

/// BT.709 luma of 8-bit RGB as a code value (0..255), rounded.
#[inline]
pub fn luma8(r: u8, g: u8, b: u8) -> u32 {
    (2126 * r as u32 + 7152 * g as u32 + 722 * b as u32 + 5000) / 10_000
}

/// The vectorscope bin (column, row) of 8-bit RGB.
#[inline]
pub fn vector_bin(r: u8, g: u8, b: u8) -> (usize, usize) {
    let y = 2126 * r as i64 + 7152 * g as i64 + 722 * b as i64; // luma × 10000 × 255
    let cb = b as i64 * 10_000 - y; // Cb × 1.8556 × 10000 × 255
    let cr = r as i64 * 10_000 - y;
    // column = floor(Cb·255 + 127.5) with Cb = cb / (18556 · 255); likewise the row from Cr, upwards.
    let col = (2 * cb + 255 * 18_556).div_euclid(2 * 18_556).clamp(0, 255) as usize;
    let up = (2 * cr + 255 * 15_748).div_euclid(2 * 15_748).clamp(0, 255) as usize;
    (col, 255 - up)
}

/// Counts of one scope: `SIZE × SIZE`, row-major.
pub fn bin(img: &Rgba, scope: Scope) -> Vec<u32> {
    let (w, h) = (img.w as usize, img.h as usize);
    if w == 0 || h == 0 {
        return vec![0; SIZE * SIZE];
    }
    // One local grid per band of rows (about 16 bands), summed at the end.
    let band = h.div_ceil(16).max(1);
    let grids: Vec<Vec<u32>> = img
        .data
        .par_chunks(w * 4 * band)
        .map(|rows| {
            let mut acc = vec![0u32; SIZE * SIZE];
            for row in rows.chunks_exact(w * 4) {
                for (x, p) in row.chunks_exact(4).enumerate() {
                    let (r, g, b) = (p[0], p[1], p[2]);
                    match scope {
                        Scope::Waveform => acc[(255 - luma8(r, g, b) as usize) * SIZE + x * SIZE / w] += 1,
                        Scope::Parade => {
                            let col = x * PARADE_PANEL / w;
                            acc[(255 - r as usize) * SIZE + col] += 1;
                            acc[(255 - g as usize) * SIZE + PARADE_PANEL + col] += 1;
                            acc[(255 - b as usize) * SIZE + 2 * PARADE_PANEL + col] += 1;
                        }
                        Scope::Vectorscope => {
                            let (c, rr) = vector_bin(r, g, b);
                            acc[rr * SIZE + c] += 1;
                        }
                    }
                }
            }
            acc
        })
        .collect();
    let mut out = vec![0u32; SIZE * SIZE];
    for g in grids {
        for (x, y) in out.iter_mut().zip(g) {
            *x += y;
        }
    }
    out
}

/// Log density of counts as u8 (the fullest bin is 255; empty bins are 0).
pub fn density(counts: &[u32]) -> Vec<u8> {
    let max = counts.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return vec![0; counts.len()];
    }
    let k = 255.0 / (1.0 + max as f64).ln();
    counts
        .iter()
        .map(|&c| if c == 0 { 0 } else { ((1.0 + c as f64).ln() * k).round().clamp(1.0, 255.0) as u8 })
        .collect()
}

/// The vectorscope bin of a colour given as 0..1 floats (target boxes, the skin line).
pub fn vector_target(c: [f32; 3]) -> (usize, usize) {
    let q = c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    vector_bin(q[0], q[1], q[2])
}

/// The 75 % colour bar colours (R, G, B in 0..1): red, yellow, green, cyan, blue, magenta.
pub const BARS_75: [[f32; 3]; 6] =
    [[0.75, 0.0, 0.0], [0.75, 0.75, 0.0], [0.0, 0.75, 0.0], [0.0, 0.75, 0.75], [0.0, 0.0, 0.75], [0.75, 0.0, 0.75]];

/// A scope as an RGBA image `SIZE × SIZE`, scaled up by `scale`: the density in green-white on black, with a
/// graticule (waveform and parade: lines at 0, 25, 50, 75 and 100 %; vectorscope: centre cross, 75 % target boxes,
/// skin line).
pub fn image(density: &[u8], scope: Scope, scale: u32) -> Rgba {
    let s = scale.max(1);
    let n = SIZE as u32 * s;
    let mut out = Rgba::filled(n, n, [12, 12, 12, 255]);
    let mut put = |x: u32, y: u32, c: [u8; 4]| {
        if x < n && y < n {
            let i = ((y * n + x) * 4) as usize;
            out.data[i..i + 4].copy_from_slice(&c);
        }
    };
    let grid = [70, 70, 70, 255];
    match scope {
        Scope::Waveform | Scope::Parade => {
            for level in [0u32, 64, 128, 191, 255] {
                for x in 0..n {
                    put(x, (255 - level) * s, grid);
                }
            }
        }
        Scope::Vectorscope => {
            for i in 0..n {
                put(i, 128 * s, grid);
                put(128 * s, i, grid);
            }
            // Skin line: from the centre through a typical skin tone to the edge.
            let (sx, sy) = vector_target([0.86, 0.64, 0.52]);
            let (dx, dy) = (sx as f32 - 127.5, sy as f32 - 127.5);
            let len = dx.hypot(dy).max(1e-3);
            for t in 0..(128 * s) {
                let f = t as f32 / s as f32;
                put(
                    ((127.5 + dx / len * f) * s as f32) as u32,
                    ((127.5 + dy / len * f) * s as f32) as u32,
                    [150, 110, 80, 255],
                );
            }
            for bar in BARS_75 {
                let (cx, cy) = vector_target(bar);
                // A box of ±2 bins around each 75 % bar target.
                let (x, y, o) = (cx as u32 * s, cy as u32 * s, 2 * s);
                for d in 0..(5 * s) {
                    put(x - o.min(x) + d, y.saturating_sub(o), [180, 60, 60, 255]);
                    put(x - o.min(x) + d, y + o, [180, 60, 60, 255]);
                    put(x.saturating_sub(o), y - o.min(y) + d, [180, 60, 60, 255]);
                    put(x + o, y - o.min(y) + d, [180, 60, 60, 255]);
                }
            }
        }
    }
    for y in 0..SIZE as u32 {
        for x in 0..SIZE as u32 {
            let d = density[(y as usize) * SIZE + x as usize];
            if d == 0 {
                continue;
            }
            let c = [d / 2, d, d / 2 + d / 4, 255];
            for yy in 0..s {
                for xx in 0..s {
                    put(x * s + xx, y * s + yy, c);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let c = BARS_75[(x * 6 / w) as usize];
                let i = ((y * w + x) * 4) as usize;
                for (d, v) in img.data[i..i + 3].iter_mut().zip(c) {
                    *d = (v * 255.0 + 0.5) as u8;
                }
                img.data[i + 3] = 255;
            }
        }
        img
    }

    /// The colour-bars scenario: each bar's vector lands in its target box within 2 bins, and the waveform shows each
    /// bar's level within 1 bin.
    #[test]
    fn colour_bars_land_on_their_targets() {
        let img = bars(960, 540);
        let v = bin(&img, Scope::Vectorscope);
        for bar in BARS_75 {
            let (tx, ty) = vector_target(bar);
            let near: u32 = (ty.saturating_sub(2)..=(ty + 2).min(255))
                .flat_map(|y| (tx.saturating_sub(2)..=(tx + 2).min(255)).map(move |x| (x, y)))
                .map(|(x, y)| v[y * SIZE + x])
                .sum();
            assert_eq!(near, 960 * 540 / 6, "{bar:?} lands in its box");
        }
        // Red at 75 %: Cb negative (left), Cr positive (up).
        let (rx, ry) = vector_target(BARS_75[0]);
        assert!(rx < 128 && ry < 128, "{rx},{ry}");
        let wf = bin(&img, Scope::Waveform);
        for (i, bar) in BARS_75.iter().enumerate() {
            let q = bar.map(|c| (c * 255.0 + 0.5) as u8);
            let level = luma8(q[0], q[1], q[2]) as usize;
            let col = (i * 960 / 6 + 80) * SIZE / 960;
            let peak = (0..SIZE).max_by_key(|&y| wf[y * SIZE + col]).unwrap();
            assert!((255 - peak).abs_diff(level) <= 1, "bar {i}: row {peak} for level {level}");
        }
        let total: u32 = bin(&img, Scope::Parade).iter().sum();
        assert_eq!(total, 3 * 960 * 540);
    }

    #[test]
    fn density_is_logarithmic_and_bounded() {
        let d = density(&[0, 1, 10, 1000]);
        assert_eq!((d[0], d[3]), (0, 255));
        assert!(d[1] >= 1 && d[1] < d[2] && d[2] < d[3]);
        assert_eq!(density(&[0, 0]), vec![0, 0]);
        let img = image(&density(&bin(&bars(96, 54), Scope::Vectorscope)), Scope::Vectorscope, 2);
        assert_eq!((img.w, img.h), (512, 512));
        assert_eq!(Scope::parse("histogram2"), None);
    }

    /// Cost at 540p (design: ≤ 6 ms on the CPU). Printed, and checked loosely so slow CI machines pass.
    #[test]
    fn binning_cost_at_540p() {
        let img = crate::parity::mosaic(960, 540);
        let _ = bin(&img, Scope::Waveform);
        for scope in Scope::ALL {
            let t = std::time::Instant::now();
            let n = 10;
            for _ in 0..n {
                std::hint::black_box(bin(&img, scope));
            }
            let ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;
            eprintln!("{scope:?}: {ms:.2} ms per 540p frame (CPU)");
            assert!(ms < 60.0, "{scope:?} {ms} ms");
        }
    }
}
