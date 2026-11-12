//! HDR → SDR for PQ (SMPTE ST 2084) and HLG (ARIB STD-B67) sources with BT.2020 primaries.
//!
//! Pipeline, per pixel:
//! 1. Y'CbCr (10–16-bit) → non-linear R'G'B' with the source matrix (chroma interpolated to full resolution).
//! 2. Each channel → *relative scene light*, 1.0 = HDR reference white (203 cd/m², ITU-R BT.2408):
//!    - PQ: ST 2084 EOTF to cd/m², divided by 203, then the inverse of the reference OOTF's system gamma (1/1.2), so
//!      the 26 cd/m² grey card lands on 0.18 like the HLG grey card does.
//!    - HLG: the inverse OETF (scene light, 0–1) divided by its reference-white value (75 % signal = 0.2647).
//! 3. Gamut: BT.2020 → BT.709 primaries in linear light (3×3 matrix), negative components clipped.
//! 4. Tone curve on max(R, G, B), applied as a gain so hue is kept: the ITU-R BT.2390 EETF (hermite knee in the PQ
//!    domain) mapping the nominal source peak (1000 cd/m², i.e. 3.78× reference white in relative scene light — HLG's
//!    peak is the same 3.78×) onto SDR white. Everything below ~0.5× reference white is untouched, so mid grey stays
//!    at 0.18; reference white lands at ~0.8 (≈ 89 % signal); the source peak lands on 100 %.
//! 5. BT.709 OETF → 8-bit full-range R'G'B'.
//!
//! The CPU path builds three small lookup tables once (signal → relative light, max → gain, linear → 8-bit code) so
//! the per-pixel work is a few multiplies, two square roots and lookups, run in parallel over rows. The viewer's WGSL
//! path (`ove-gpu`) evaluates the same math exactly; tests check the two agree.

use std::sync::OnceLock;

use rayon::prelude::*;

use crate::convert::Transfer;

/// HDR reference white in cd/m² (ITU-R BT.2408); maps to relative scene light 1.0.
pub const REFERENCE_WHITE_NITS: f64 = 203.0;
/// Peak luminance assumed for PQ sources without usable mastering metadata (cd/m²).
pub const PQ_NOMINAL_PEAK_NITS: f64 = 1000.0;
/// Reference OOTF system gamma between scene and display light (BT.2100 / BT.2408).
pub const SYSTEM_GAMMA: f64 = 1.2;

/// Linear BT.2020 RGB → linear BT.709 RGB (ITU-R BT.2087).
pub const BT2020_TO_BT709: [[f64; 3]; 3] = [
    [1.660_491_0, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;

const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 0.284_668_92;
const HLG_C: f64 = 0.559_910_73;

/// SMPTE ST 2084 EOTF: signal 0–1 → cd/m².
pub fn pq_eotf(e: f64) -> f64 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    10000.0 * ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1)
}

/// Inverse of [`pq_eotf`]: cd/m² → signal 0–1.
pub fn pq_inverse_eotf(nits: f64) -> f64 {
    let y = (nits / 10000.0).clamp(0.0, 1.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
}

/// ARIB STD-B67 inverse OETF: signal 0–1 → scene light 0–1.
pub fn hlg_inverse_oetf(e: f64) -> f64 {
    let e = e.clamp(0.0, 1.0);
    if e <= 0.5 { e * e / 3.0 } else { (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0 }
}

/// ITU-R BT.709 OETF: linear 0–1 → signal 0–1.
pub fn bt709_oetf(l: f64) -> f64 {
    let l = l.clamp(0.0, 1.0);
    if l < 0.018 { 4.5 * l } else { 1.099 * l.powf(0.45) - 0.099 }
}

/// A channel's signal (0–1) → relative scene light (1.0 = reference white). SDR passes through linearised with the
/// BT.709 inverse OETF's power approximation (not used by the tone-mapping path).
pub fn to_relative_scene(transfer: Transfer, e: f64) -> f64 {
    match transfer {
        Transfer::Pq => (pq_eotf(e) / REFERENCE_WHITE_NITS).powf(1.0 / SYSTEM_GAMMA),
        Transfer::Hlg => hlg_inverse_oetf(e) / hlg_inverse_oetf(0.75),
        Transfer::Sdr => e.clamp(0.0, 1.0).powf(1.0 / 0.45),
    }
}

/// Nominal source peak in relative scene light (≈ 3.78 for both PQ at 1000 cd/m² and HLG).
pub fn source_peak() -> f64 {
    (PQ_NOMINAL_PEAK_NITS / REFERENCE_WHITE_NITS).powf(1.0 / SYSTEM_GAMMA)
}

/// Constants of the BT.2390 EETF as used here: (PQ signal of the source peak, target max in normalised PQ, knee
/// start in normalised PQ). Shared with the GPU path.
pub fn eetf_constants() -> (f64, f64, f64) {
    let src_pq = pq_inverse_eotf(source_peak() * REFERENCE_WHITE_NITS);
    let max_lum = pq_inverse_eotf(REFERENCE_WHITE_NITS) / src_pq;
    let ks = 1.5 * max_lum - 0.5;
    (src_pq, max_lum, ks)
}

/// The tone curve: relative scene light `m` (max of R, G, B) → SDR linear light 0–1 (1.0 = SDR white).
pub fn tone_curve(m: f64) -> f64 {
    if m <= 0.0 {
        return 0.0;
    }
    let (src_pq, max_lum, ks) = eetf_constants();
    let e1 = (pq_inverse_eotf(m * REFERENCE_WHITE_NITS) / src_pq).min(1.0);
    let e2 = if e1 < ks {
        e1
    } else {
        let t = (e1 - ks) / (1.0 - ks);
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * ks + (t3 - 2.0 * t2 + t) * (1.0 - ks) + (-2.0 * t3 + 3.0 * t2) * max_lum
    };
    (pq_eotf(e2 * src_pq) / REFERENCE_WHITE_NITS).min(1.0)
}

/// Reference conversion of one non-linear BT.2020 R'G'B' pixel (0–1) to SDR BT.709 R'G'B' (0–1), in f64 with no
/// tables. The CPU tables and the GPU shader are tested against this.
pub fn reference_rgb(transfer: Transfer, rgb: [f64; 3], bt2020_primaries: bool) -> [f64; 3] {
    let lin = rgb.map(|c| to_relative_scene(transfer, c));
    let mut out = if bt2020_primaries {
        let m = &BT2020_TO_BT709;
        [0, 1, 2].map(|i| (m[i][0] * lin[0] + m[i][1] * lin[1] + m[i][2] * lin[2]).max(0.0))
    } else {
        lin
    };
    let max = out[0].max(out[1]).max(out[2]);
    if max > 0.0 {
        let gain = tone_curve(max) / max;
        out = out.map(|c| c * gain);
    }
    out.map(bt709_oetf)
}

const LIN_SIZE: usize = 4096;

// The gain and OETF tables are indexed by the bits of an f32: exponent and top mantissa bits give a log-spaced index
// with integer operations only (no square roots or logarithms per pixel).
/// Gain table: from 0.25 (below the knee) to 64 (a 10 000 cd/m² red after gamut mapping is 25.7 × 1.66), 256 entries
/// per octave, linearly interpolated.
const GAIN_LO: f32 = 0.25;
const GAIN_HI: f32 = 64.0;
const GAIN_SHIFT: u32 = 15;
const GAIN_SIZE: usize = (8 << 8) + 2;
/// OETF table: 2^-16 (below half a code value) to 1.0, 512 entries per octave: < 0.15 code between entries.
const OETF_LO: f32 = 1.0 / 65536.0;
const OETF_SHIFT: u32 = 14;
const OETF_SIZE: usize = (16 << 9) + 2;

/// Lookup tables for one transfer function.
pub(crate) struct Tables {
    /// Signal (12-bit index) → relative scene light.
    lin: Box<[f32; LIN_SIZE]>,
    /// log-spaced max(R, G, B) → gain tone_curve(m) / m.
    gain: Box<[f32; GAIN_SIZE]>,
    /// Relative light at which the curve starts compressing (gain is 1 below it).
    knee: f32,
    /// log-spaced SDR linear light → 8-bit code (rounded at the midpoint between entries).
    oetf: Box<[u8; OETF_SIZE]>,
}

/// The value at log-spaced table index `i` above `lo` with `shift` mantissa bits dropped.
fn log_entry(lo: f32, shift: u32, i: usize) -> f64 {
    f64::from(f32::from_bits(lo.to_bits() + ((i as u32) << shift)))
}

impl Tables {
    fn new(transfer: Transfer) -> Tables {
        let mut lin = Box::new([0f32; LIN_SIZE]);
        for (i, v) in lin.iter_mut().enumerate() {
            *v = to_relative_scene(transfer, i as f64 / (LIN_SIZE - 1) as f64) as f32;
        }
        let mut gain = Box::new([0f32; GAIN_SIZE]);
        for (i, g) in gain.iter_mut().enumerate() {
            let m = log_entry(GAIN_LO, GAIN_SHIFT, i);
            *g = (tone_curve(m) / m) as f32;
        }
        let mut oetf = Box::new([0u8; OETF_SIZE]);
        for (i, c) in oetf.iter_mut().enumerate() {
            // Entry i covers [v_i, v_i+1): store the code of its midpoint.
            let v = 0.5 * (log_entry(OETF_LO, OETF_SHIFT, i) + log_entry(OETF_LO, OETF_SHIFT, i + 1));
            *c = (bt709_oetf(v) * 255.0).round() as u8;
        }
        let (src_pq, _, ks) = eetf_constants();
        let knee = ((pq_eotf(ks * src_pq) / REFERENCE_WHITE_NITS) * 0.999).max(f64::from(GAIN_LO)) as f32;
        Tables { lin, gain, knee, oetf }
    }

    pub(crate) fn get(transfer: Transfer) -> &'static Tables {
        static PQ: OnceLock<Tables> = OnceLock::new();
        static HLG: OnceLock<Tables> = OnceLock::new();
        match transfer {
            Transfer::Hlg => HLG.get_or_init(|| Tables::new(Transfer::Hlg)),
            _ => PQ.get_or_init(|| Tables::new(Transfer::Pq)),
        }
    }

    #[inline(always)]
    fn linear(&self, signal: f32) -> f32 {
        let i = (signal * (LIN_SIZE - 1) as f32 + 0.5) as i32;
        self.lin[i.clamp(0, LIN_SIZE as i32 - 1) as usize]
    }

    #[inline(always)]
    fn gain(&self, m: f32) -> f32 {
        if m <= self.knee {
            return 1.0;
        }
        let d = m.min(GAIN_HI).to_bits() - GAIN_LO.to_bits();
        let i = ((d >> GAIN_SHIFT) as usize).min(GAIN_SIZE - 2);
        let f = (d & ((1 << GAIN_SHIFT) - 1)) as f32 * (1.0 / (1 << GAIN_SHIFT) as f32);
        self.gain[i] + (self.gain[i + 1] - self.gain[i]) * f
    }

    #[inline(always)]
    fn code(&self, l: f32) -> u8 {
        // As i32, float bits order like the floats (negatives, -0.0 included, are below every positive value), so
        // clamping the bits clamps the value to [LO, 1].
        let lo = OETF_LO.to_bits() as i32;
        let b = (l.to_bits() as i32).clamp(lo, 1f32.to_bits() as i32) - lo;
        self.oetf[((b >> OETF_SHIFT) as usize).min(OETF_SIZE - 1)]
    }

    /// Non-linear R'G'B' (0–1, source primaries) → 8-bit SDR BT.709 R'G'B'. `m` is the gamut matrix (identity for
    /// BT.709 primaries).
    #[inline(always)]
    pub(crate) fn pixel(&self, r: f32, g: f32, b: f32, m: &[[f32; 3]; 3]) -> [u8; 3] {
        let (r, g, b) = (self.linear(r), self.linear(g), self.linear(b));
        let rr = (m[0][0] * r + m[0][1] * g + m[0][2] * b).max(0.0);
        let gg = (m[1][0] * r + m[1][1] * g + m[1][2] * b).max(0.0);
        let bb = (m[2][0] * r + m[2][1] * g + m[2][2] * b).max(0.0);
        let k = self.gain(rr.max(gg).max(bb));
        [self.code(rr * k), self.code(gg * k), self.code(bb * k)]
    }
}

/// The gamut matrix for the tables: BT.2020 → BT.709, or identity.
pub(crate) fn gamut_matrix(bt2020_primaries: bool) -> [[f32; 3]; 3] {
    if bt2020_primaries {
        BT2020_TO_BT709.map(|r| r.map(|v| v as f32))
    } else {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }
}

/// Y'CbCr → R'G'B' coefficients and code ranges, everything scaled to 16-bit code values.
#[derive(Debug, Clone, Copy)]
pub struct YuvParams {
    /// (Kr, Kb) of the matrix.
    pub kr: f32,
    pub kb: f32,
    pub full_range: bool,
}

/// One source picture in 16-bit-scaled planes. Samples are read as little-endian u16 and scaled by
/// `2^(16 - depth)`, so every depth uses the same ranges.
#[derive(Debug, Clone, Copy)]
pub struct YuvSource<'a> {
    pub width: usize,
    pub height: usize,
    pub depth: u32,
    /// log2 chroma subsampling (0 or 1).
    pub log2_chroma_w: u32,
    pub log2_chroma_h: u32,
    pub y: (&'a [u8], usize),
    /// Planar: Cb and Cr planes. Semi-planar: `cb` holds interleaved CbCr and `cr` is `None`.
    pub cb: (&'a [u8], usize),
    pub cr: Option<(&'a [u8], usize)>,
    /// Samples stored in the high bits (P010/P016 layout): read as-is, no scaling.
    pub msb_aligned: bool,
}

#[inline(always)]
fn le16(b: &[u8]) -> f32 {
    f32::from(u16::from_le_bytes([b[0], b[1]]))
}

impl YuvSource<'_> {
    fn scale(&self) -> f32 {
        if self.msb_aligned { 1.0 } else { (1u32 << (16 - self.depth)) as f32 }
    }

    /// Adds `weight ×` chroma row `cy` (16-bit scaled) to `cb` and `cr` (one value per chroma sample).
    fn add_chroma_row(&self, cy: usize, weight: f32, cb: &mut [f32], cr: &mut [f32]) {
        let k = weight * self.scale();
        let n = cb.len();
        match self.cr {
            Some((crp, crs)) => {
                let rb = &self.cb.0[cy * self.cb.1..][..n * 2];
                let rr = &crp[cy * crs..][..n * 2];
                for (((ob, or), b), r) in
                    cb.iter_mut().zip(cr.iter_mut()).zip(rb.chunks_exact(2)).zip(rr.chunks_exact(2))
                {
                    *ob += k * le16(b);
                    *or += k * le16(r);
                }
            }
            None => {
                let row = &self.cb.0[cy * self.cb.1..][..n * 4];
                for ((ob, or), c) in cb.iter_mut().zip(cr.iter_mut()).zip(row.chunks_exact(4)) {
                    *ob += k * le16(&c[..2]);
                    *or += k * le16(&c[2..]);
                }
            }
        }
    }
}

/// Tone-maps a 10–16-bit Y'CbCr picture into tightly packed RGBA8 (`out.len() == width * height * 4`).
///
/// Chroma is upsampled with linear interpolation (co-sited horizontally, centred vertically: the MPEG-2/H.264/HEVC
/// default). `bt2020_primaries` selects the BT.2020 → BT.709 gamut conversion.
pub fn tone_map_yuv(src: &YuvSource<'_>, p: YuvParams, transfer: Transfer, bt2020_primaries: bool, out: &mut [u8]) {
    let (w, h) = (src.width, src.height);
    assert_eq!(out.len(), w * h * 4, "output size");
    if w == 0 || h == 0 {
        return;
    }
    let tables = Tables::get(transfer);
    let gamut = gamut_matrix(bt2020_primaries);
    let (yoff, yrange, cmid, crange) =
        if p.full_range { (0.0, 65535.0, 32768.0, 65535.0) } else { (4096.0, 56064.0, 32768.0, 57344.0) };
    let (kr, kb) = (p.kr, p.kb);
    let kg = 1.0 - kr - kb;
    let (rv, gu, gv, bu) =
        (2.0 * (1.0 - kr), -2.0 * kb * (1.0 - kb) / kg, -2.0 * kr * (1.0 - kr) / kg, 2.0 * (1.0 - kb));
    let (sx, sy) = (src.log2_chroma_w as usize, src.log2_chroma_h as usize);
    let (cw, ch) = ((w + sx) >> sx, (h + sy) >> sy);
    let (ys, yo) = (src.scale() / yrange, -yoff / yrange);
    let (cs, co) = (1.0 / crange, -cmid / crange);

    out.par_chunks_exact_mut(w * 4).enumerate().for_each_init(
        || (vec![0f32; cw], vec![0f32; cw], vec![0f32; w], vec![0f32; w]),
        |(vcb, vcr, hcb, hcr), (row, dst)| {
            // Vertical chroma interpolation into one chroma-width row.
            vcb.fill(0.0);
            vcr.fill(0.0);
            if sy == 1 {
                let cy = row / 2;
                let other = if row % 2 == 0 { cy.saturating_sub(1) } else { (cy + 1).min(ch - 1) };
                src.add_chroma_row(cy, 0.75, vcb, vcr);
                src.add_chroma_row(other, 0.25, vcb, vcr);
            } else {
                src.add_chroma_row(row, 1.0, vcb, vcr);
            }
            // Horizontal interpolation to full width, normalised to -0.5..0.5.
            if sx == 1 {
                for (i, (pb, pr)) in hcb.chunks_mut(2).zip(hcr.chunks_mut(2)).enumerate() {
                    let j = (i + 1).min(cw - 1);
                    pb[0] = vcb[i] * cs + co;
                    pr[0] = vcr[i] * cs + co;
                    if pb.len() == 2 {
                        pb[1] = 0.5 * (vcb[i] + vcb[j]) * cs + co;
                        pr[1] = 0.5 * (vcr[i] + vcr[j]) * cs + co;
                    }
                }
            } else {
                for (o, v) in hcb.iter_mut().zip(&*vcb) {
                    *o = v * cs + co;
                }
                for (o, v) in hcr.iter_mut().zip(&*vcr) {
                    *o = v * cs + co;
                }
            }
            let yrow = &src.y.0[row * src.y.1..][..w * 2];
            for (((px, yb), &cb), &cr) in dst.chunks_exact_mut(4).zip(yrow.chunks_exact(2)).zip(&*hcb).zip(&*hcr) {
                let yn = le16(yb) * ys + yo;
                let rgb = tables.pixel(yn + rv * cr, yn + gu * cb + gv * cr, yn + bu * cb, &gamut);
                px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_functions_round_trip_and_hit_reference_points() {
        for nits in [0.1, 26.0, 100.0, 203.0, 1000.0, 4000.0] {
            assert!((pq_eotf(pq_inverse_eotf(nits)) - nits).abs() / nits < 1e-9, "{nits}");
        }
        // BT.2408 reference levels: 203 cd/m² ≈ 58 % PQ, HLG 75 % = reference white.
        assert!((pq_inverse_eotf(203.0) - 0.58).abs() < 0.005);
        assert!((hlg_inverse_oetf(0.75) - 0.2647).abs() < 0.001);
        assert!((hlg_inverse_oetf(1.0) - 1.0).abs() < 1e-6);
        assert!((bt709_oetf(0.18) - 0.409).abs() < 0.001);
    }

    #[test]
    fn grey_card_and_reference_white_land_in_sdr_range() {
        for (t, grey, white) in
            [(Transfer::Pq, pq_inverse_eotf(26.0), pq_inverse_eotf(203.0)), (Transfer::Hlg, 0.38, 0.75)]
        {
            let g = reference_rgb(t, [grey; 3], true)[0] * 255.0;
            let w = reference_rgb(t, [white; 3], true)[0] * 255.0;
            // SDR 18 % grey through the BT.709 OETF is code 104.
            assert!((g - 104.0).abs() < 4.0, "{t:?} grey {g}");
            assert!((215.0..=240.0).contains(&w), "{t:?} white {w}");
            let peak = reference_rgb(t, [1.0; 3], true)[0] * 255.0;
            assert!(peak > 253.0, "{t:?} peak {peak}");
        }
    }

    #[test]
    fn curve_is_identity_below_the_knee_and_monotonic() {
        assert!((tone_curve(0.18) - 0.18).abs() < 1e-9);
        assert!((tone_curve(source_peak()) - 1.0).abs() < 1e-3);
        let mut last = 0.0;
        for i in 1..2000 {
            let v = tone_curve(i as f64 * 0.01);
            assert!(v >= last - 1e-12, "monotonic at {i}");
            last = v;
        }
        assert!(last <= 1.0);
    }

    #[test]
    fn tables_match_the_reference_within_one_code() {
        for t in [Transfer::Pq, Transfer::Hlg] {
            let tables = Tables::get(t);
            let gamut = gamut_matrix(true);
            let mut worst = 0i32;
            for r in (0..=1023).step_by(31) {
                for g in (0..=1023).step_by(37) {
                    for b in (0..=1023).step_by(41) {
                        let rgb = [r, g, b].map(|v| v as f64 / 1023.0);
                        let want = reference_rgb(t, rgb, true).map(|v| (v * 255.0).round() as i32);
                        let got = tables.pixel(rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, &gamut);
                        for c in 0..3 {
                            worst = worst.max((got[c] as i32 - want[c]).abs());
                        }
                    }
                }
            }
            assert!(worst <= 1, "{t:?}: worst {worst}");
        }
    }
}
