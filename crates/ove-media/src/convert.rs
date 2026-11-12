//! Fast planar YUV → NV12 / P010 repacking for the viewer.
//!
//! Every planar YUV format without alpha (4:2:0, 4:2:2, 4:4:0, 4:4:4; 8 to 16 bits) is repacked into the two
//! semi-planar formats the GPU uploads, without colour conversion: luma is copied (shifted to the high bits for
//! P010), chroma is interleaved and, for 4:2:2/4:4:4 sources, subsampled to 4:2:0 by taking every other sample.
//! That is a preview simplification (full chroma is kept for export by the compositor in change 6), and it is ~20×
//! faster than a general swscale conversion at 4K. Range and matrix are carried in [`ColorInfo`].

use ffmpeg_the_third::ffi;
use serde::Serialize;

use crate::frame::{PixelFormat, Plane};

/// YUV → RGB matrix of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorMatrix {
    Bt601,
    Bt709,
    Bt2020,
}

/// Transfer function (EOTF/OETF) of picture data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Transfer {
    /// BT.709 / sRGB-like SDR.
    #[default]
    Sdr,
    /// SMPTE ST 2084 (HDR10, Dolby Vision base layers).
    Pq,
    /// ARIB STD-B67 hybrid log-gamma (phones, broadcast).
    Hlg,
}

/// Colour description the viewer needs to convert YUV correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorInfo {
    pub matrix: ColorMatrix,
    /// Full range (0–255) instead of video range (16–235).
    pub full_range: bool,
    /// The source's transfer is PQ or HLG (HDR), whether or not this frame was tone-mapped.
    pub hdr: bool,
    /// Transfer of this frame's pixels. `Pq`/`Hlg` only for untouched HDR data ([`crate::OutputFormat::DisplayHdr`],
    /// [`crate::OutputFormat::Native`]); RGBA output of an HDR source is tone-mapped to SDR BT.709 and says `Sdr`.
    pub transfer: Transfer,
    /// Primaries are BT.2020 (the gamut a GPU tone-mapper converts from).
    pub bt2020_primaries: bool,
}

impl Default for ColorInfo {
    fn default() -> Self {
        ColorInfo {
            matrix: ColorMatrix::Bt709,
            full_range: false,
            hdr: false,
            transfer: Transfer::Sdr,
            bt2020_primaries: false,
        }
    }
}

impl ColorMatrix {
    /// (Kr, Kb) luma coefficients.
    pub fn coefficients(self) -> (f32, f32) {
        match self {
            ColorMatrix::Bt601 => (0.299, 0.114),
            ColorMatrix::Bt709 => (0.2126, 0.0722),
            ColorMatrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// Colour info of a decoded frame (unspecified matrix: BT.709 for HD and up, BT.601 below).
pub(crate) unsafe fn color_info(f: *const ffi::AVFrame) -> ColorInfo {
    // SAFETY: caller passes a valid frame.
    unsafe {
        let matrix = match (*f).colorspace {
            ffi::AVColorSpace::BT709 => ColorMatrix::Bt709,
            ffi::AVColorSpace::BT2020_NCL | ffi::AVColorSpace::BT2020_CL => ColorMatrix::Bt2020,
            ffi::AVColorSpace::BT470BG | ffi::AVColorSpace::SMPTE170M | ffi::AVColorSpace::FCC => ColorMatrix::Bt601,
            _ if (*f).height >= 720 => ColorMatrix::Bt709,
            _ => ColorMatrix::Bt601,
        };
        let transfer = match (*f).color_trc {
            ffi::AVColorTransferCharacteristic::SMPTE2084 => Transfer::Pq,
            ffi::AVColorTransferCharacteristic::ARIB_STD_B67 => Transfer::Hlg,
            _ => Transfer::Sdr,
        };
        // Unspecified primaries on an HDR stream are BT.2020 in practice.
        let bt2020_primaries = (*f).color_primaries == ffi::AVColorPrimaries::BT2020
            || ((*f).color_primaries == ffi::AVColorPrimaries::UNSPECIFIED && transfer != Transfer::Sdr);
        ColorInfo {
            matrix,
            full_range: (*f).color_range == ffi::AVColorRange::JPEG,
            hdr: transfer != Transfer::Sdr,
            transfer,
            bt2020_primaries,
        }
    }
}

/// The frame's planes as a 10–16-bit Y'CbCr source for the tone-mapper, or `None` for layouts it does not read
/// (8-bit, RGB, alpha, big-endian, packed, …) — the caller converts those to `yuv444p16le` with swscale first.
pub(crate) unsafe fn yuv_source<'a>(f: *const ffi::AVFrame) -> Option<crate::tonemap::YuvSource<'a>> {
    // SAFETY: caller passes a valid, CPU-mapped frame that outlives the returned borrow; each slice covers
    // linesize × rows of its plane.
    unsafe {
        let fmt = ffi::AVPixelFormat((*f).format);
        let d = ffi::av_pix_fmt_desc_get(fmt);
        if d.is_null() {
            return None;
        }
        let d = &*d;
        let unsupported = (ffi::AV_PIX_FMT_FLAG_RGB
            | ffi::AV_PIX_FMT_FLAG_ALPHA
            | ffi::AV_PIX_FMT_FLAG_BE
            | ffi::AV_PIX_FMT_FLAG_PAL
            | ffi::AV_PIX_FMT_FLAG_HWACCEL
            | ffi::AV_PIX_FMT_FLAG_BITSTREAM
            | ffi::AV_PIX_FMT_FLAG_FLOAT) as u64;
        let planes = ffi::av_pix_fmt_count_planes(fmt);
        if d.flags & unsupported != 0
            || d.nb_components != 3
            || d.log2_chroma_w > 1
            || d.log2_chroma_h > 1
            || d.comp[0].step != 2
            || d.comp[0].depth <= 8
        {
            return None;
        }
        let (w, h) = ((*f).width as usize, (*f).height as usize);
        let rows = |i: usize, sub: u8| if i == 0 { h } else { (h + (1 << sub) - 1) >> sub };
        let plane = |i: usize| -> Option<(&'a [u8], usize)> {
            let ls = (*f).linesize[i];
            if ls <= 0 || (*f).data[i].is_null() {
                return None;
            }
            let n = rows(i, d.log2_chroma_h);
            Some((std::slice::from_raw_parts((*f).data[i], ls as usize * n), ls as usize))
        };
        let (planar, msb_aligned) = match planes {
            // Planar: samples in the low bits.
            3 if d.comp[0].shift == 0 && d.comp[1].plane == 1 && d.comp[2].plane == 2 => (true, false),
            // Semi-planar P010/P016/P210/P216: interleaved CbCr, samples in the high bits.
            2 if d.comp[1].plane == 1 && d.comp[2].plane == 1 && d.comp[1].step == 4 && d.comp[1].offset == 0 => {
                (false, true)
            }
            _ => return None,
        };
        // msb-aligned layouts store `depth` bits shifted up by `shift`; together they fill the word.
        if msb_aligned && d.comp[0].shift as u32 + d.comp[0].depth as u32 != 16 {
            return None;
        }
        Some(crate::tonemap::YuvSource {
            width: w,
            height: h,
            depth: d.comp[0].depth as u32,
            log2_chroma_w: d.log2_chroma_w as u32,
            log2_chroma_h: d.log2_chroma_h as u32,
            y: plane(0)?,
            cb: plane(1)?,
            cr: if planar { Some(plane(2)?) } else { None },
            msb_aligned,
        })
    }
}

/// Repack a planar YUV frame into NV12 (8-bit sources) or P010 (9–16-bit). `None` if the layout is not handled
/// (RGB, alpha, big-endian, packed, semi-planar, palettes) — the caller falls back to swscale.
pub(crate) unsafe fn to_semiplanar(f: *const ffi::AVFrame) -> Option<(PixelFormat, Vec<Plane>)> {
    // SAFETY: caller passes a valid, CPU-mapped frame; reads stay within each plane's linesize × rows.
    unsafe {
        let fmt = ffi::AVPixelFormat((*f).format);
        let d = ffi::av_pix_fmt_desc_get(fmt);
        if d.is_null() {
            return None;
        }
        let d = &*d;
        let flags = d.flags;
        let unsupported = (ffi::AV_PIX_FMT_FLAG_RGB
            | ffi::AV_PIX_FMT_FLAG_ALPHA
            | ffi::AV_PIX_FMT_FLAG_BE
            | ffi::AV_PIX_FMT_FLAG_PAL
            | ffi::AV_PIX_FMT_FLAG_HWACCEL
            | ffi::AV_PIX_FMT_FLAG_BITSTREAM
            | ffi::AV_PIX_FMT_FLAG_FLOAT) as u64;
        if flags & unsupported != 0
            || flags & ffi::AV_PIX_FMT_FLAG_PLANAR as u64 == 0
            || d.nb_components != 3
            || ffi::av_pix_fmt_count_planes(fmt) != 3
            || d.log2_chroma_w > 1
            || d.log2_chroma_h > 1
        {
            return None;
        }
        let depth = d.comp[0].depth as u32;
        let bytes = if depth > 8 { 2usize } else { 1 };
        if d.comp[0].step as usize != bytes || d.comp[0].shift != 0 {
            return None;
        }
        let (w, h) = ((*f).width as usize, (*f).height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let (sx, sy) = (1 - d.log2_chroma_w as usize, 1 - d.log2_chroma_h as usize);
        // Source chroma samples read per row (last one at index (cw - 1) << sx).
        let clen = ((cw - 1) << sx) + 1;
        let plane = |i: usize| ((*f).data[i] as *const u8, (*f).linesize[i] as isize);
        let (yp, yls) = plane(0);
        let (up, uls) = plane(1);
        let (vp, vls) = plane(2);

        if bytes == 1 {
            let mut y = vec![0u8; w * h];
            for (r, row) in y.chunks_exact_mut(w).enumerate() {
                row.copy_from_slice(std::slice::from_raw_parts(yp.offset(r as isize * yls), w));
            }
            let mut uv = vec![0u8; cw * 2 * ch];
            for (r, row) in uv.chunks_exact_mut(cw * 2).enumerate() {
                let su = std::slice::from_raw_parts(up.offset((r << sy) as isize * uls), clen);
                let sv = std::slice::from_raw_parts(vp.offset((r << sy) as isize * vls), clen);
                for c in 0..cw {
                    row[2 * c] = su[c << sx];
                    row[2 * c + 1] = sv[c << sx];
                }
            }
            Some((
                PixelFormat::Nv12,
                vec![
                    Plane { data: y, stride: w, width: w as u32, height: h as u32 },
                    Plane { data: uv, stride: cw * 2, width: cw as u32, height: ch as u32 },
                ],
            ))
        } else {
            let shift = 16 - depth;
            let mut y = vec![0u8; w * h * 2];
            for (r, row) in y.chunks_exact_mut(w * 2).enumerate() {
                let src = std::slice::from_raw_parts(yp.offset(r as isize * yls) as *const u16, w);
                for (o, s) in row.chunks_exact_mut(2).zip(src) {
                    o.copy_from_slice(&(s << shift).to_le_bytes());
                }
            }
            let mut uv = vec![0u8; cw * 4 * ch];
            for (r, row) in uv.chunks_exact_mut(cw * 4).enumerate() {
                let su = std::slice::from_raw_parts(up.offset((r << sy) as isize * uls) as *const u16, clen);
                let sv = std::slice::from_raw_parts(vp.offset((r << sy) as isize * vls) as *const u16, clen);
                for c in 0..cw {
                    row[4 * c..4 * c + 2].copy_from_slice(&(su[c << sx] << shift).to_le_bytes());
                    row[4 * c + 2..4 * c + 4].copy_from_slice(&(sv[c << sx] << shift).to_le_bytes());
                }
            }
            Some((
                PixelFormat::P010,
                vec![
                    Plane { data: y, stride: w * 2, width: w as u32, height: h as u32 },
                    Plane { data: uv, stride: cw * 4, width: cw as u32, height: ch as u32 },
                ],
            ))
        }
    }
}
