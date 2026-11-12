//! CPU reference path: YUV → RGB conversion and letterboxed composition.
//!
//! Used as the oracle in GPU tests and as the fallback when no GPU adapter exists. The math is the
//! same as the GPU shaders (nearest chroma sample, same range and matrix constants, same HDR tone
//! mapping).

use crate::fit::fit_rect;
use crate::frame::{PixelFormat, Transfer, VideoFrame, YuvMatrix};

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

// HDR → SDR, mirroring the WGSL `tone_map` and `ove_media::tonemap` (the reference those are tested
// against). Relative scene light: 1.0 = HDR reference white (203 cd/m², BT.2408).
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;
const REF_WHITE: f32 = 203.0;
/// HLG inverse OETF of 75 % (HDR reference white).
const HLG_REF_SCENE: f32 = 0.264_962_56;
const SYSTEM_GAMMA: f32 = 1.2;
/// Peak assumed for PQ sources (cd/m²); HLG's nominal peak is the same in relative scene light.
const PQ_PEAK: f32 = 1000.0;
/// Linear BT.2020 RGB → linear BT.709 RGB (ITU-R BT.2087).
const BT2020_TO_BT709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

fn pq_eotf(e: f32) -> f32 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    10000.0 * ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1)
}

fn pq_inverse(nits: f32) -> f32 {
    let y = (nits / 10000.0).clamp(0.0, 1.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
}

fn hlg_inverse(e: f32) -> f32 {
    let e = e.clamp(0.0, 1.0);
    if e <= 0.5 { e * e / 3.0 } else { (((e - 0.559_910_7) / 0.178_832_77).exp() + 0.284_668_92) / 12.0 }
}

fn bt709_oetf(l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    if l < 0.018 { 4.5 * l } else { 1.099 * l.powf(0.45) - 0.099 }
}

/// Shader uniforms for a transfer: `[mode, PQ signal of the source peak, target max, knee start]`
/// (mode 0 = no tone mapping, 1 = PQ, 2 = HLG) and the linear-light gamut matrix.
pub fn tone_params(transfer: Transfer) -> ([f32; 4], [[f32; 3]; 3]) {
    let src_pq = pq_inverse(REF_WHITE * (PQ_PEAK / REF_WHITE).powf(1.0 / SYSTEM_GAMMA));
    let max_lum = pq_inverse(REF_WHITE) / src_pq;
    let ks = 1.5 * max_lum - 0.5;
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    match transfer {
        Transfer::Sdr => ([0.0, src_pq, max_lum, ks], identity),
        Transfer::Pq => ([1.0, src_pq, max_lum, ks], BT2020_TO_BT709),
        Transfer::Hlg => ([2.0, src_pq, max_lum, ks], BT2020_TO_BT709),
    }
}

/// BT.2390 EETF on relative scene light → SDR linear light 0..1.
fn tone_curve(m: f32, tone: [f32; 4]) -> f32 {
    let [_, src_pq, max_lum, ks] = tone;
    let e1 = (pq_inverse(m * REF_WHITE) / src_pq).min(1.0);
    let e2 = if e1 < ks {
        e1
    } else {
        let t = (e1 - ks) / (1.0 - ks);
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * ks + (t3 - 2.0 * t2 + t) * (1.0 - ks) + (-2.0 * t3 + 3.0 * t2) * max_lum
    };
    (pq_eotf(e2 * src_pq) / REF_WHITE).min(1.0)
}

/// Non-linear R'G'B' (0..1) of a `transfer` source → SDR BT.709 R'G'B' (0..1). Identity for SDR.
pub fn tone_map_rgb(transfer: Transfer, rgb: [f32; 3]) -> [f32; 3] {
    let (tone, g) = tone_params(transfer);
    let lin = match transfer {
        Transfer::Sdr => return rgb,
        Transfer::Pq => rgb.map(|c| (pq_eotf(c) / REF_WHITE).powf(1.0 / SYSTEM_GAMMA)),
        Transfer::Hlg => rgb.map(|c| hlg_inverse(c) / HLG_REF_SCENE),
    };
    let out = [0, 1, 2].map(|i| (g[i][0] * lin[0] + g[i][1] * lin[1] + g[i][2] * lin[2]).max(0.0));
    let m = out[0].max(out[1]).max(out[2]);
    let k = if m > 0.0 { tone_curve(m, tone) / m } else { 1.0 };
    out.map(|c| bt709_oetf(c * k))
}

fn read_u16(plane: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([plane[offset], plane[offset + 1]])
}

/// Converts a frame to tightly packed RGBA8 at its own resolution.
pub fn frame_to_rgba(frame: &VideoFrame) -> Vec<u8> {
    frame_to_rgba_with_transfer(frame, Transfer::Sdr)
}

/// [`frame_to_rgba`] for a YUV frame of the given transfer (PQ / HLG are tone-mapped to SDR).
pub fn frame_to_rgba_with_transfer(frame: &VideoFrame, transfer: Transfer) -> Vec<u8> {
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
                    let rgb = tone_map_rgb(transfer, yuv_code_to_rgb(y, u, v, params, &m));
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
    compose_letterboxed_with_transfer(frame, Transfer::Sdr, target_w, target_h, background)
}

/// [`compose_letterboxed`] for a YUV frame of the given transfer (PQ / HLG are tone-mapped to SDR).
pub fn compose_letterboxed_with_transfer(
    frame: &VideoFrame,
    transfer: Transfer,
    target_w: u32,
    target_h: u32,
    background: [u8; 4],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(target_w as usize * target_h as usize * 4);
    for _ in 0..(target_w as usize * target_h as usize) {
        out.extend_from_slice(&background);
    }
    let fit = fit_rect(frame.width, frame.height, target_w, target_h);
    if fit.width == 0 || fit.height == 0 {
        return out;
    }
    let src = frame_to_rgba_with_transfer(frame, transfer);
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
    fn hdr_grey_card_and_reference_white_map_to_sdr() {
        let code = |t, v: f32| (tone_map_rgb(t, [v; 3])[0] * 255.0).round();
        // BT.2408: grey card at 38 % HLG / 26 cd/m² PQ, reference white at 75 % HLG / 203 cd/m² PQ.
        for (t, grey, white) in [(Transfer::Hlg, 0.38, 0.75), (Transfer::Pq, pq_inverse(26.0), pq_inverse(203.0))] {
            assert!((code(t, grey) - 104.0).abs() <= 3.0, "{t:?} grey {}", code(t, grey));
            assert!((215.0..=240.0).contains(&code(t, white)), "{t:?} white {}", code(t, white));
            assert_eq!(code(t, 1.0), 255.0, "{t:?} peak");
        }
        assert_eq!(tone_map_rgb(Transfer::Sdr, [0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]);
    }

    #[test]
    fn compose_pillarboxes() {
        let frame = VideoFrame::rgba(1, 1, vec![255, 0, 0, 255], 0);
        let out = compose_letterboxed(&frame, 3, 1, [0, 0, 0, 255]);
        assert_eq!(out, vec![0, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255]);
    }
}
