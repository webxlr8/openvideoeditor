//! CPU reference path: YUV → RGB conversion and letterboxed composition.
//!
//! Used as the oracle in GPU tests and as the fallback when no GPU adapter exists. The math is the
//! same as the GPU shaders (nearest chroma sample, same range and matrix constants).

use crate::fit::fit_rect;
use crate::frame::{PixelFormat, VideoFrame, YuvMatrix};

/// Range parameters for a bit depth: (luma offset, luma range, chroma midpoint, chroma range).
pub fn range_params(bits: u32, full_range: bool) -> [f32; 4] {
    let max = ((1u32 << bits) - 1) as f32;
    let scale = (1u32 << (bits - 8)) as f32;
    if full_range {
        [0.0, max, (1u32 << (bits - 1)) as f32, max]
    } else {
        [16.0 * scale, 219.0 * scale, 128.0 * scale, 224.0 * scale]
    }
}

/// Row-major 3×3 Y'CbCr → R'G'B' matrix for normalised Y in 0..1 and Cb/Cr in -0.5..0.5.
pub fn yuv_matrix(matrix: YuvMatrix) -> [[f32; 3]; 3] {
    let (kr, kb) = matrix.coefficients();
    let kg = 1.0 - kr - kb;
    [
        [1.0, 0.0, 2.0 * (1.0 - kr)],
        [1.0, -2.0 * kb * (1.0 - kb) / kg, -2.0 * kr * (1.0 - kr) / kg],
        [1.0, 2.0 * (1.0 - kb), 0.0],
    ]
}

/// Converts one sample given as integer code values.
pub fn yuv_code_to_rgb(y: f32, u: f32, v: f32, params: [f32; 4], m: &[[f32; 3]; 3]) -> [f32; 3] {
    let yn = (y - params[0]) / params[1];
    let cb = (u - params[2]) / params[3];
    let cr = (v - params[2]) / params[3];
    let mut out = [0.0; 3];
    for (i, row) in m.iter().enumerate() {
        out[i] = (row[0] * yn + row[1] * cb + row[2] * cr).clamp(0.0, 1.0);
    }
    out
}

fn read_u16(plane: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([plane[offset], plane[offset + 1]])
}

/// Converts a frame to tightly packed RGBA8 at its own resolution.
pub fn frame_to_rgba(frame: &VideoFrame) -> Vec<u8> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut out = vec![0u8; w * h * 4];
    match frame.format {
        PixelFormat::Rgba8 => {
            let stride = frame.strides[0] as usize;
            for row in 0..h {
                out[row * w * 4..(row + 1) * w * 4]
                    .copy_from_slice(&frame.planes[0][row * stride..row * stride + w * 4]);
            }
        }
        PixelFormat::Nv12 | PixelFormat::P010 => {
            let ten = frame.format == PixelFormat::P010;
            let params = range_params(if ten { 10 } else { 8 }, frame.full_range);
            let m = yuv_matrix(frame.matrix);
            let (ys, uvs) = (frame.strides[0] as usize, frame.strides[1] as usize);
            for row in 0..h {
                for col in 0..w {
                    let (cx, cy) = (col / 2, row / 2);
                    let (y, u, v) = if ten {
                        (
                            f32::from(read_u16(&frame.planes[0], row * ys + col * 2) >> 6),
                            f32::from(read_u16(&frame.planes[1], cy * uvs + cx * 4) >> 6),
                            f32::from(read_u16(&frame.planes[1], cy * uvs + cx * 4 + 2) >> 6),
                        )
                    } else {
                        (
                            f32::from(frame.planes[0][row * ys + col]),
                            f32::from(frame.planes[1][cy * uvs + cx * 2]),
                            f32::from(frame.planes[1][cy * uvs + cx * 2 + 1]),
                        )
                    };
                    let rgb = yuv_code_to_rgb(y, u, v, params, &m);
                    let o = (row * w + col) * 4;
                    for c in 0..3 {
                        out[o + c] = (rgb[c] * 255.0).round() as u8;
                    }
                    out[o + 3] = 255;
                }
            }
        }
    }
    out
}

/// Composes `frame` letterboxed into a `target_w`×`target_h` RGBA8 image over `background`
/// (nearest-neighbour scaling). This is the software fallback presenter.
pub fn compose_letterboxed(frame: &VideoFrame, target_w: u32, target_h: u32, background: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(target_w as usize * target_h as usize * 4);
    for _ in 0..(target_w as usize * target_h as usize) {
        out.extend_from_slice(&background);
    }
    let fit = fit_rect(frame.width, frame.height, target_w, target_h);
    if fit.width == 0 || fit.height == 0 {
        return out;
    }
    let src = frame_to_rgba(frame);
    for ty in 0..fit.height {
        let sy = (u64::from(ty) * u64::from(frame.height) / u64::from(fit.height)) as usize;
        for tx in 0..fit.width {
            let sx = (u64::from(tx) * u64::from(frame.width) / u64::from(fit.width)) as usize;
            let s = (sy * frame.width as usize + sx) * 4;
            let d = ((fit.y as usize + ty as usize) * target_w as usize + fit.x as usize + tx as usize) * 4;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_range_extremes() {
        let m = yuv_matrix(YuvMatrix::Bt709);
        let p = range_params(8, false);
        assert_eq!(yuv_code_to_rgb(16.0, 128.0, 128.0, p, &m), [0.0, 0.0, 0.0]);
        assert_eq!(yuv_code_to_rgb(235.0, 128.0, 128.0, p, &m), [1.0, 1.0, 1.0]);
        let p10 = range_params(10, false);
        assert_eq!(yuv_code_to_rgb(940.0, 512.0, 512.0, p10, &m), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn bt709_red() {
        // Limited-range BT.709 red: Y=63, Cb=102, Cr=240.
        let rgb = yuv_code_to_rgb(63.0, 102.0, 240.0, range_params(8, false), &yuv_matrix(YuvMatrix::Bt709));
        assert!(rgb[0] > 0.98 && rgb[1] < 0.02 && rgb[2] < 0.02, "{rgb:?}");
    }

    #[test]
    fn compose_pillarboxes() {
        let frame = VideoFrame::rgba(1, 1, vec![255, 0, 0, 255], 0);
        let out = compose_letterboxed(&frame, 3, 1, [0, 0, 0, 255]);
        assert_eq!(out, vec![0, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255]);
    }
}
