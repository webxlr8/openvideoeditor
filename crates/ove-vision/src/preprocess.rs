//! Image preprocessing for the image encoder: resize to 256 × 256 (aspect not kept, as the model was trained), scale
//! to 0–1, then normalise each channel with mean 0.5 and deviation 0.5, laid out channel by channel (`[3, 256, 256]`).
//!
//! The resize is a separable triangle (bilinear) filter whose support widens with the scale factor when shrinking, so
//! every source pixel contributes (an antialiased bilinear resize); the result is rounded to 8 bits before scaling,
//! as an 8-bit image library would.

/// The encoder's input side in pixels.
pub const IMAGE_SIZE: usize = 256;

/// Values per preprocessed image.
pub const IMAGE_VALUES: usize = 3 * IMAGE_SIZE * IMAGE_SIZE;

/// Filter taps for one output coordinate: the first source index and the normalised weights.
fn taps(in_size: usize, out_size: usize) -> Vec<(usize, Vec<f64>)> {
    let scale = in_size as f64 / out_size as f64;
    let filter_scale = scale.max(1.0);
    let support = filter_scale;
    (0..out_size)
        .map(|x| {
            let center = (x as f64 + 0.5) * scale;
            let lo = ((center - support + 0.5).floor().max(0.0)) as usize;
            let hi = ((center + support + 0.5).floor() as usize).min(in_size);
            let mut w: Vec<f64> = (lo..hi)
                .map(|i| {
                    let d = ((i as f64 - center + 0.5) / filter_scale).abs();
                    if d < 1.0 { 1.0 - d } else { 0.0 }
                })
                .collect();
            let sum: f64 = w.iter().sum();
            if sum > 0.0 {
                for v in &mut w {
                    *v /= sum;
                }
            }
            (lo, w)
        })
        .collect()
}

/// Resizes an RGBA image (alpha ignored) to `out_w` × `out_h` RGB, 8 bits per channel.
pub fn resize_rgb(rgba: &[u8], width: usize, height: usize, out_w: usize, out_h: usize) -> Vec<u8> {
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return vec![0; out_w * out_h * 3];
    }
    let tx = taps(width, out_w);
    let ty = taps(height, out_h);
    // Horizontal pass into floats, then vertical pass with rounding once at the end.
    let mut mid = vec![0f64; out_w * height * 3];
    for y in 0..height {
        let row = &rgba[y * width * 4..(y + 1) * width * 4];
        for (x, (lo, w)) in tx.iter().enumerate() {
            let mut acc = [0f64; 3];
            for (k, wk) in w.iter().enumerate() {
                let p = &row[(lo + k) * 4..(lo + k) * 4 + 3];
                for c in 0..3 {
                    acc[c] += wk * f64::from(p[c]);
                }
            }
            mid[(y * out_w + x) * 3..(y * out_w + x) * 3 + 3].copy_from_slice(&acc);
        }
    }
    let mut out = vec![0u8; out_w * out_h * 3];
    for (y, (lo, w)) in ty.iter().enumerate() {
        for x in 0..out_w {
            let mut acc = [0f64; 3];
            for (k, wk) in w.iter().enumerate() {
                let i = ((lo + k) * out_w + x) * 3;
                for c in 0..3 {
                    acc[c] += wk * mid[i + c];
                }
            }
            for c in 0..3 {
                out[(y * out_w + x) * 3 + c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The encoder input for one RGBA image: `[3, 256, 256]` floats in −1…1.
pub fn pixel_values(rgba: &[u8], width: usize, height: usize) -> Vec<f32> {
    let rgb = resize_rgb(rgba, width, height, IMAGE_SIZE, IMAGE_SIZE);
    let plane = IMAGE_SIZE * IMAGE_SIZE;
    let mut out = vec![0f32; IMAGE_VALUES];
    for (i, p) in rgb.chunks_exact(3).enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (f32::from(p[c]) / 255.0 - 0.5) / 0.5;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, c: [u8; 3]) -> Vec<u8> {
        (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect()
    }

    #[test]
    fn a_solid_image_stays_solid_and_is_normalised() {
        let v = pixel_values(&solid(640, 360, [255, 0, 128]), 640, 360);
        assert_eq!(v.len(), IMAGE_VALUES);
        let plane = IMAGE_SIZE * IMAGE_SIZE;
        assert!(v[..plane].iter().all(|&x| x == 1.0));
        assert!(v[plane..2 * plane].iter().all(|&x| x == -1.0));
        let mid = (128.0 / 255.0 - 0.5) / 0.5;
        assert!(v[2 * plane..].iter().all(|&x| (x - mid).abs() < 1e-6));
    }

    #[test]
    fn halving_averages_neighbours() {
        // A 4 × 1 row 0, 100, 200, 0 halved to 2 × 1: each output covers two pixels plus a quarter of each neighbour,
        // weights 1/8, 3/8, 3/8, 1/8 (edges renormalised).
        let rgba: Vec<u8> = [0u8, 100, 200, 0].iter().flat_map(|&v| [v, v, v, 255]).collect();
        let out = resize_rgb(&rgba, 4, 1, 2, 1);
        // Left: taps 0..3 with weights 3/8, 3/8, 1/8 renormalised over 7/8 → (0*3 + 100*3 + 200*1)/7 = 71.43.
        assert_eq!(out[0], 71);
        // Right: taps 1..4 with weights 1/8, 3/8, 3/8 → (100*1 + 200*3 + 0*3)/7 = 100.
        assert_eq!(out[3], 100);
    }

    #[test]
    fn enlarging_interpolates() {
        let rgba: Vec<u8> = [0u8, 255].iter().flat_map(|&v| [v, v, v, 255]).collect();
        let out = resize_rgb(&rgba, 2, 1, 4, 1);
        assert_eq!(out.iter().step_by(3).copied().collect::<Vec<_>>(), [0, 64, 191, 255]);
    }

    #[test]
    fn an_empty_image_is_black() {
        assert!(resize_rgb(&[], 0, 0, 2, 2).iter().all(|&v| v == 0));
    }
}
