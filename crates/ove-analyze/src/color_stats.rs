//! Numeric colour readout of one frame, for agents that match shots: luma statistics, clipping, zone colours,
//! CIELAB balance, saturation, a saturation-weighted hue histogram and skin-line deviation. With a reference frame it
//! adds the per-zone CIEDE2000 gap and a capped grade patch (wheels and saturation) that moves the shot toward the
//! reference.
//!
//! Conventions (display-referred, 8-bit code values as exported):
//! - Luma is BT.709 (`0.2126 R + 0.7152 G + 0.0722 B`) in percent of full scale (0..100).
//! - Zones split pixels by luma: shadows below 33.3 %, highlights from 66.7 %, midtones between.
//! - Clipped low: every channel at 2/255 or below. Clipped high: any channel at 253/255 or above.
//! - Saturation is HSV saturation (0..1). Hue bins are 30° wide and centred on 0° (red), 30°, 60° (yellow) … 330°.
//! - Colourful pixels have saturation ≥ 0.25 and a brightest channel ≥ 15 %.
//! - The skin line is the conventional 123° on a BT.709 vectorscope (angle of (Cb, Cr) from +Cb). Pixels within 30°
//!   of it, with some chroma and mid luma, count as skin-like; the deviation is their chroma-weighted mean angle
//!   minus 123° (positive: toward yellow, negative: toward red and magenta).
//!
//! Sums run in a fixed order in integers or f64 and every number is rounded, so the same pixels always give
//! byte-identical JSON.

use serde::Serialize;
use serde_json::{Map, Value, json};

use ove_render::Rgba;
pub use ove_render::color::{delta_e00, lab_u8};
use ove_render::color::{lab_of_linear, lin_table};

/// Vectorscope angle of the skin line, degrees from +Cb toward +Cr.
pub const SKIN_LINE_DEG: f64 = 123.0;
/// Half-width of the skin-like band around the skin line, degrees.
const SKIN_BAND_DEG: f64 = 30.0;
/// Below this share of skin-like pixels (percent) there is no skin-line deviation.
const SKIN_MIN_SHARE: f64 = 0.5;
/// Zone shares below this (percent) are too small to compare.
const ZONE_MIN_SHARE: f64 = 0.5;

/// Per-step limits of a suggested correction (on top of the shot's current grade).
const MAX_GAIN_STEP: f64 = 0.35;
const MAX_OFFSET_STEP: f64 = 0.25;
const SAT_STEP: (f64, f64) = (0.75, 1.33);

/// Luma scale: `2126 R + 7152 G + 722 B` for 8-bit channels runs 0..=2_550_000.
const Y_FULL: u64 = 2_550_000;
/// Percentile resolution: 1021 bins of 2500 (0.1 % of full scale each, near enough).
const Y_BIN: u64 = 2_500;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LumaStats {
    /// Percent of full scale.
    pub mean: f64,
    pub p2: f64,
    pub p98: f64,
    /// 16 equal bins over 0..100 %, percent of pixels in each.
    pub histogram: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clipping {
    /// Percent of pixels.
    pub low: f64,
    pub high: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Zone {
    /// Percent of pixels in this zone.
    pub share: f64,
    /// Mean colour, 0..1 per channel.
    pub rgb: [f64; 3],
    /// Mean CIELAB (D65) colour.
    pub lab: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Zones {
    pub shadows: Option<Zone>,
    pub midtones: Option<Zone>,
    pub highlights: Option<Zone>,
}

impl Zones {
    fn all(&self) -> [(&'static str, Option<&Zone>); 3] {
        [
            ("shadows", self.shadows.as_ref()),
            ("midtones", self.midtones.as_ref()),
            ("highlights", self.highlights.as_ref()),
        ]
    }

    /// Share-weighted mean Lab of the whole frame.
    fn mean_lab(&self) -> Option<[f64; 3]> {
        let mut acc = [0.0; 3];
        let mut w = 0.0;
        for (_, z) in self.all() {
            if let Some(z) = z {
                for (a, l) in acc.iter_mut().zip(z.lab) {
                    *a += l * z.share;
                }
                w += z.share;
            }
        }
        (w > 0.0).then(|| acc.map(|v| v / w))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    /// Mean CIELAB a* (negative green, positive magenta).
    pub a: f64,
    /// Mean CIELAB b* (negative blue, positive yellow).
    pub b: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    /// Percent of pixels near the skin line.
    pub share: f64,
    /// Degrees from the skin line (positive toward yellow); none when too few pixels are skin-like.
    pub deviation: Option<f64>,
}

/// The readout of one frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorStats {
    /// Pixels measured.
    pub pixels: u64,
    pub luma: LumaStats,
    pub clipping: Clipping,
    pub zones: Zones,
    pub balance: Balance,
    /// Mean HSV saturation, 0..1.
    pub saturation: f64,
    /// 12 hue bins (0°, 30° … 330°), percent of the total saturation weight.
    pub hue: Vec<f64>,
    /// Percent of colourful pixels.
    pub colorful: f64,
    pub skin: Skin,
}

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    let r = (v * k).round() / k;
    if r == 0.0 { 0.0 } else { r } // no "-0.0"
}

fn r2(v: f64) -> f64 {
    round(v, 2)
}

fn r3(v: f64) -> f64 {
    round(v, 3)
}

fn r4(v: f64) -> f64 {
    round(v, 4)
}

#[derive(Default)]
struct ZoneAcc {
    n: u64,
    rgb: [u64; 3],
    lab: [f64; 3],
}

impl ZoneAcc {
    fn finish(&self, total: u64) -> Option<Zone> {
        (self.n > 0).then(|| {
            let n = self.n as f64;
            Zone {
                share: r2(100.0 * n / total as f64),
                rgb: self.rgb.map(|v| r4(v as f64 / n / 255.0)),
                lab: self.lab.map(|v| r3(v / n)),
            }
        })
    }
}

/// Pixels to measure: all, or only those at least half opaque (`masked`, for an item rendered alone).
fn measured(img: &Rgba, masked: bool) -> impl Iterator<Item = [u8; 3]> + '_ {
    img.data.chunks_exact(4).filter(move |p| !masked || p[3] >= 128).map(|p| [p[0], p[1], p[2]])
}

/// The readout of `img`. With `masked`, pixels less than half opaque are left out.
pub fn color_stats(img: &Rgba, masked: bool) -> ColorStats {
    let lin = lin_table();
    let mut n = 0u64;
    let mut y_sum = 0u64;
    let mut y_bins = vec![0u64; (Y_FULL / Y_BIN + 1) as usize];
    let mut hist16 = [0u64; 16];
    let (mut clip_lo, mut clip_hi) = (0u64, 0u64);
    let mut zones: [ZoneAcc; 3] = Default::default();
    let (mut a_sum, mut b_sum) = (0.0f64, 0.0f64);
    let mut sat_sum = 0.0f64;
    let mut hue = [0.0f64; 12];
    let mut colorful = 0u64;
    let (mut skin_n, mut skin_w, mut skin_dev) = (0u64, 0.0f64, 0.0f64);
    for [r, g, b] in measured(img, masked) {
        n += 1;
        let y = 2126 * r as u64 + 7152 * g as u64 + 722 * b as u64;
        y_sum += y;
        y_bins[(y / Y_BIN) as usize] += 1;
        hist16[((y * 16) / (Y_FULL + 1)) as usize] += 1;
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        if mx <= 2 {
            clip_lo += 1;
        }
        if mx >= 253 {
            clip_hi += 1;
        }
        let lab = lab_of_linear(lin[r as usize], lin[g as usize], lin[b as usize]);
        a_sum += lab[1];
        b_sum += lab[2];
        let z = if y * 3 < Y_FULL {
            0
        } else if y * 3 < Y_FULL * 2 {
            1
        } else {
            2
        };
        let acc = &mut zones[z];
        acc.n += 1;
        acc.rgb[0] += r as u64;
        acc.rgb[1] += g as u64;
        acc.rgb[2] += b as u64;
        for (a, l) in acc.lab.iter_mut().zip(lab) {
            *a += l;
        }
        // HSV saturation and hue.
        let sat = if mx == 0 { 0.0 } else { (mx - mn) as f64 / mx as f64 };
        sat_sum += sat;
        if mx > mn {
            let (rf, gf, bf, d) = (r as f64, g as f64, b as f64, (mx - mn) as f64);
            let h = if mx == r {
                60.0 * ((gf - bf) / d).rem_euclid(6.0)
            } else if mx == g {
                60.0 * ((bf - rf) / d + 2.0)
            } else {
                60.0 * ((rf - gf) / d + 4.0)
            };
            let bin = (((h + 15.0).rem_euclid(360.0)) / 30.0) as usize;
            hue[bin.min(11)] += sat;
        }
        if sat >= 0.25 && mx >= 38 {
            colorful += 1;
        }
        // Vectorscope position (BT.709 on code values).
        let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
        let yl = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
        let (cb, cr) = ((bf - yl) / 1.8556, (rf - yl) / 1.5748);
        let chroma = cb.hypot(cr);
        if chroma >= 0.02 && (0.1..=0.95).contains(&yl) {
            let ang = cr.atan2(cb).to_degrees().rem_euclid(360.0);
            let dev = ang - SKIN_LINE_DEG;
            if dev.abs() <= SKIN_BAND_DEG {
                skin_n += 1;
                skin_w += chroma;
                skin_dev += dev * chroma;
            }
        }
    }
    let nf = n.max(1) as f64;
    let pct = |v: u64| r2(100.0 * v as f64 / nf);
    let percentile = |p: f64| -> f64 {
        let want = (p * n as f64).ceil().max(1.0) as u64;
        let mut cum = 0u64;
        for (i, c) in y_bins.iter().enumerate() {
            cum += c;
            if cum >= want {
                return r2(100.0 * (i as u64 * Y_BIN) as f64 / Y_FULL as f64);
            }
        }
        100.0
    };
    let hue_total: f64 = hue.iter().sum();
    let skin_share = pct(skin_n);
    ColorStats {
        pixels: n,
        luma: LumaStats {
            mean: r2(100.0 * y_sum as f64 / nf / Y_FULL as f64),
            p2: if n == 0 { 0.0 } else { percentile(0.02) },
            p98: if n == 0 { 0.0 } else { percentile(0.98) },
            histogram: hist16.iter().map(|&c| pct(c)).collect(),
        },
        clipping: Clipping { low: pct(clip_lo), high: pct(clip_hi) },
        zones: Zones { shadows: zones[0].finish(n), midtones: zones[1].finish(n), highlights: zones[2].finish(n) },
        balance: Balance { a: r3(a_sum / nf), b: r3(b_sum / nf) },
        saturation: r4(sat_sum / nf),
        hue: hue.iter().map(|&h| if hue_total > 0.0 { r2(100.0 * h / hue_total) } else { 0.0 }).collect(),
        colorful: pct(colorful),
        skin: Skin {
            share: skin_share,
            deviation: (skin_share >= SKIN_MIN_SHARE && skin_w > 0.0).then(|| r2(skin_dev / skin_w)),
        },
    }
}

/// How far a shot is from a reference, in CIEDE2000.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    /// Per-zone ΔE00 between the zones' mean colours (none when a zone is missing in either frame).
    pub shadows: Option<f64>,
    pub midtones: Option<f64>,
    pub highlights: Option<f64>,
    /// Zone gaps weighted by the reference's zone shares; the gap of the frames' mean colours when no zone is in both.
    pub mean: f64,
    /// Reference minus shot: luma mean (percent) and CIELAB balance.
    pub luma: f64,
    pub a: f64,
    pub b: f64,
}

/// The gap from `shot` to `reference`.
pub fn gap(shot: &ColorStats, reference: &ColorStats) -> Gap {
    let mut out = [None; 3];
    let (mut sum, mut w) = (0.0, 0.0);
    for (k, ((_, s), (_, r))) in shot.zones.all().into_iter().zip(reference.zones.all()).enumerate() {
        if let (Some(s), Some(r)) = (s, r)
            && s.share >= ZONE_MIN_SHARE
            && r.share >= ZONE_MIN_SHARE
        {
            let d = delta_e00(s.lab, r.lab);
            out[k] = Some(r3(d));
            sum += d * r.share;
            w += r.share;
        }
    }
    let mean = if w > 0.0 {
        sum / w
    } else {
        match (shot.zones.mean_lab(), reference.zones.mean_lab()) {
            (Some(a), Some(b)) => delta_e00(a, b),
            _ => 0.0,
        }
    };
    Gap {
        shadows: out[0],
        midtones: out[1],
        highlights: out[2],
        mean: r3(mean),
        luma: r2(reference.luma.mean - shot.luma.mean),
        a: r3(reference.balance.a - shot.balance.a),
        b: r3(reference.balance.b - shot.balance.b),
    }
}

/// The measured pixels of `img` as one opaque row (for fits that must not see transparent areas).
pub fn opaque_strip(img: &Rgba, masked: bool) -> Rgba {
    let data: Vec<u8> = measured(img, masked).flat_map(|[r, g, b]| [r, g, b, 255]).collect();
    Rgba { w: (data.len() / 4) as u32, h: 1, data }
}

/// A wheel as `[r, g, b, y]` from its JSON (object or array); zeros when absent.
fn wheel(v: Option<&Value>) -> [f64; 4] {
    let n = |x: Option<&Value>| x.and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0);
    match v {
        Some(Value::Object(o)) => [n(o.get("r")), n(o.get("g")), n(o.get("b")), n(o.get("y"))],
        Some(Value::Array(a)) => [n(a.first()), n(a.get(1)), n(a.get(2)), n(a.get(3))],
        _ => [0.0; 4],
    }
}

fn wheel_json(w: [f64; 4]) -> Value {
    json!({"r": r4(w[0]), "g": r4(w[1]), "b": r4(w[2]), "y": r4(w[3])})
}

/// Mean CIELAB chroma of some pixels.
fn mean_chroma(img: &Rgba) -> f64 {
    let lin = lin_table();
    let mut sum = 0.0;
    let mut n = 0u64;
    for p in img.data.chunks_exact(4) {
        let l = lab_of_linear(lin[p[0] as usize], lin[p[1] as usize], lin[p[2] as usize]);
        sum += l[1].hypot(l[2]);
        n += 1;
    }
    sum / n.max(1) as f64
}

/// Per-channel 256-bin histograms of some opaque pixels.
fn channel_hist(img: &Rgba) -> [[u64; 256]; 3] {
    let mut h = [[0u64; 256]; 3];
    for p in img.data.chunks_exact(4) {
        for c in 0..3 {
            h[c][p[c] as usize] += 1;
        }
    }
    h
}

/// The code value at quantile `q` of a histogram (the smallest value with at least `q` of the pixels at or below).
fn quantile(h: &[u64; 256], q: f64) -> u8 {
    let n: u64 = h.iter().sum();
    let want = (q * n as f64).ceil().max(1.0) as u64;
    let mut cum = 0u64;
    for (v, c) in h.iter().enumerate() {
        cum += c;
        if cum >= want {
            return v as u8;
        }
    }
    255
}

/// Per channel, the line `ref ≈ a·shot + b` through matching quantiles (5 % … 95 %) of the two frames, leaving out
/// clipped values, as (gain, offset) wheel values: `(x + o/2)(1 + g) = a·x + b`. Scaled toward identity by `k`.
fn quantile_fit(reference: &Rgba, shot: &Rgba, k: f64) -> ([f64; 3], [f64; 3]) {
    let (hr, hs) = (channel_hist(reference), channel_hist(shot));
    let (mut gain, mut offset) = ([0.0; 3], [0.0; 3]);
    for c in 0..3 {
        let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for i in 1..=19 {
            let q = i as f64 * 0.05;
            let (x, y) = (quantile(&hs[c], q), quantile(&hr[c], q));
            if x <= 1 || x >= 254 || y <= 1 || y >= 254 {
                continue;
            }
            let (x, y) = (x as f64 / 255.0, y as f64 / 255.0);
            sx += x;
            sy += y;
            sxx += x * x;
            sxy += x * y;
            n += 1.0;
        }
        let den = n * sxx - sx * sx;
        let (a, b) = if n >= 2.0 && den.abs() > 1e-9 {
            let a = ((n * sxy - sx * sy) / den).clamp(0.2, 4.0);
            (a, (sy - a * sx) / n)
        } else if n >= 1.0 {
            (1.0, (sy - sx) / n) // a flat channel: shift only
        } else {
            (1.0, 0.0)
        };
        let (a, b) = (1.0 + (a - 1.0) * k, b * k);
        gain[c] = a - 1.0;
        offset[c] = 2.0 * b / a;
    }
    (gain, offset)
}

/// A new `colorGrade` object for a shot whose current look is `shot` (opaque pixels), so it moves toward
/// `reference`: the shot's current grade (`current`, the item's `colorGrade` or none) with its gain and offset
/// wheels composed with a fitted correction, and its saturation scaled to match the reference's chroma. The
/// correction is a per-channel line through matching quantiles of the two frames. Each call is one capped step
/// (gain ±0.35, offset ±0.25, saturation ×0.75…1.33 per channel and step), scaled by `strength` (0..1), so repeated
/// inspect-and-apply rounds converge without overshooting. Lift, gamma, curves and LUTs are kept.
pub fn suggest_grade(reference: &Rgba, shot: &Rgba, current: Option<&Value>, strength: f64) -> Value {
    let k = strength.clamp(0.0, 1.0);
    let (fit_g, fit_o) = quantile_fit(reference, shot, k);
    let step_g = fit_g.map(|g| g.clamp(-MAX_GAIN_STEP, MAX_GAIN_STEP));
    let step_o = fit_o.map(|o| o.clamp(-MAX_OFFSET_STEP, MAX_OFFSET_STEP));
    let mut grade: Map<String, Value> = current.and_then(Value::as_object).cloned().unwrap_or_default();
    grade.remove("match");
    let wheels = grade.get("wheels").and_then(Value::as_object).cloned().unwrap_or_default();
    let (g1, o1) = (wheel(wheels.get("gain")), wheel(wheels.get("offset")));
    // The correction h(v) = (v + o₂/2)(1 + g₂) after the current f(x) = (x + o₁/2)(1 + g₁) is
    // (x + o₁/2 + o₂/(2(1 + g₁)))(1 + g₁)(1 + g₂): compose per channel, keeping each wheel's master (y).
    let (mut gain, mut offset) = (g1, o1);
    for c in 0..3 {
        let total_g1 = g1[c] + g1[3];
        let total_o1 = o1[c] + o1[3];
        let new_g = (1.0 + total_g1) * (1.0 + step_g[c]) - 1.0;
        let new_o = total_o1 + step_o[c] / (1.0 + total_g1).max(0.05);
        gain[c] = new_g - g1[3];
        offset[c] = new_o - o1[3];
    }
    // Saturation: the reference's chroma over the corrected shot's.
    let mut moved = shot.clone();
    for p in moved.data.chunks_exact_mut(4) {
        for c in 0..3 {
            let v = (p[c] as f64 / 255.0 + step_o[c] * 0.5) * (1.0 + step_g[c]);
            p[c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    let (rc, mc) = (mean_chroma(reference), mean_chroma(&moved));
    let old_sat = grade.get("saturation").and_then(Value::as_f64).filter(|s| s.is_finite()).unwrap_or(1.0);
    let step_s = if rc > 1.0 && mc > 1.0 { (1.0 + (rc / mc - 1.0) * k).clamp(SAT_STEP.0, SAT_STEP.1) } else { 1.0 };
    let sat = (old_sat * step_s).clamp(0.0, 4.0);
    let mut w = Map::new();
    w.insert("lift".into(), wheel_json(wheel(wheels.get("lift"))));
    w.insert("gamma".into(), wheel_json(wheel(wheels.get("gamma"))));
    w.insert("gain".into(), wheel_json(gain));
    w.insert("offset".into(), wheel_json(offset));
    grade.insert("wheels".into(), Value::Object(w));
    grade.insert("saturation".into(), json!(r3(sat)));
    Value::Object(grade)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Rgba {
        let mut out = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                let i = ((y * w + x) * 4) as usize;
                out.data[i..i + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
        out
    }

    /// 75 % colour bars: white, yellow, cyan, green, magenta, red, blue.
    fn bars(w: u32, h: u32) -> Rgba {
        const B: [[u8; 3]; 7] =
            [[191, 191, 191], [191, 191, 0], [0, 191, 191], [0, 191, 0], [191, 0, 191], [191, 0, 0], [0, 0, 191]];
        img(w, h, |x, _| B[(x * 7 / w) as usize])
    }

    #[test]
    fn grey_ramp_luma_and_zones() {
        // 256 columns, one per code value.
        let ramp = img(256, 4, |x, _| [x as u8; 3]);
        let s = color_stats(&ramp, false);
        assert_eq!(s.pixels, 1024);
        assert!((s.luma.mean - 50.0).abs() < 0.01, "{}", s.luma.mean);
        assert!((s.luma.p2 - 2.0).abs() < 0.5, "{}", s.luma.p2);
        assert!((s.luma.p98 - 98.0).abs() < 0.5, "{}", s.luma.p98);
        assert_eq!(s.luma.histogram.len(), 16);
        for h in &s.luma.histogram {
            assert!((h - 6.25).abs() < 0.01, "even ramp → even bins: {:?}", s.luma.histogram);
        }
        assert!((s.clipping.low - 300.0 / 256.0).abs() < 0.01, "codes 0..=2");
        assert!((s.clipping.high - 300.0 / 256.0).abs() < 0.01, "codes 253..=255");
        // Grey has no colour.
        assert_eq!(s.saturation, 0.0);
        assert!(s.hue.iter().all(|&h| h == 0.0));
        assert_eq!(s.colorful, 0.0);
        assert!(s.balance.a.abs() < 0.01 && s.balance.b.abs() < 0.01, "{:?}", s.balance);
        let shares: f64 = s.zones.all().iter().filter_map(|(_, z)| z.map(|z| z.share)).sum();
        assert!((shares - 100.0).abs() < 0.02);
        let mid = s.zones.midtones.as_ref().unwrap();
        assert!((mid.rgb[0] - 0.5).abs() < 0.01 && mid.rgb[0] == mid.rgb[2]);
        assert!(mid.lab[1].abs() < 0.01);
        assert_eq!(s.skin.deviation, None);
    }

    #[test]
    fn colour_bars_hue_and_saturation() {
        let s = color_stats(&bars(700, 10), false);
        // Six coloured bars of seven are fully saturated.
        assert!((s.saturation - 6.0 / 7.0).abs() < 0.001, "{}", s.saturation);
        assert!((s.colorful - 600.0 / 7.0).abs() < 0.01, "{}", s.colorful);
        // Equal weight in the six primary and secondary bins (0, 60, 120, 180, 240, 300 degrees).
        for (i, h) in s.hue.iter().enumerate() {
            let want = if i % 2 == 0 { 100.0 / 6.0 } else { 0.0 };
            assert!((h - want).abs() < 0.02, "bin {i}: {:?}", s.hue);
        }
        assert_eq!(s.clipping.high, 0.0, "75% bars do not clip");
    }

    #[test]
    fn warm_and_cool_casts_move_balance() {
        let base = |x: u32, y: u32| [(60 + x) as u8, (70 + y) as u8, (80 + (x + y) / 2) as u8];
        let neutral = color_stats(&img(128, 128, base), false);
        let warm = color_stats(
            &img(128, 128, |x, y| {
                let [r, g, b] = base(x, y);
                [r.saturating_add(30), g.saturating_add(8), b.saturating_sub(25)]
            }),
            false,
        );
        let cool = color_stats(
            &img(128, 128, |x, y| {
                let [r, g, b] = base(x, y);
                [r.saturating_sub(25), g, b.saturating_add(30)]
            }),
            false,
        );
        assert!(
            warm.balance.b > neutral.balance.b + 10.0,
            "warm is yellower: {:?} {:?}",
            warm.balance,
            neutral.balance
        );
        assert!(warm.balance.a > neutral.balance.a, "warm is redder");
        assert!(cool.balance.b < neutral.balance.b - 10.0, "cool is bluer");
        let g = gap(&warm, &neutral);
        assert!(g.b < -10.0 && g.mean > 5.0, "{g:?}");
        assert_eq!(gap(&neutral, &neutral).mean, 0.0);
    }

    #[test]
    fn skin_line_deviation() {
        // A typical skin tone sits close to the line; pushing it toward red gives a negative deviation.
        let skin = color_stats(&img(32, 32, |_, _| [224, 172, 140]), false);
        let d = skin.skin.deviation.expect("skin pixels");
        assert!(d.abs() < 8.0, "{d}");
        assert_eq!(skin.skin.share, 100.0);
        let red = color_stats(&img(32, 32, |_, _| [224, 150, 150]), false);
        assert!(red.skin.deviation.unwrap() < d - 5.0, "{:?}", red.skin);
        let yellow = color_stats(&img(32, 32, |_, _| [214, 180, 130]), false);
        assert!(yellow.skin.deviation.unwrap() > d + 5.0, "{:?}", yellow.skin);
        // Blue is far from the line.
        let blue = color_stats(&img(32, 32, |_, _| [40, 60, 200]), false);
        assert_eq!(blue.skin.share, 0.0);
        assert_eq!(blue.skin.deviation, None);
    }

    #[test]
    fn mask_leaves_out_transparent_pixels() {
        let mut half = img(10, 10, |_, _| [200, 200, 200]);
        for p in half.data.chunks_exact_mut(4).take(50) {
            p.copy_from_slice(&[0, 0, 0, 0]);
        }
        assert_eq!(color_stats(&half, true).pixels, 50);
        assert!((color_stats(&half, true).luma.mean - 78.43).abs() < 0.01);
        assert_eq!(color_stats(&half, false).pixels, 100);
        assert_eq!(opaque_strip(&half, true).w, 50);
        let empty = color_stats(&Rgba::new(4, 4), true);
        assert_eq!(empty.pixels, 0);
        assert_eq!(empty.luma.mean, 0.0);
    }

    #[test]
    fn output_is_deterministic_json() {
        let a = serde_json::to_string(&color_stats(&bars(320, 90), false)).unwrap();
        let b = serde_json::to_string(&color_stats(&bars(320, 90), false)).unwrap();
        assert_eq!(a, b);
        assert!(!a.contains("-0.0"));
    }

    #[test]
    fn ciede2000_reference_pairs() {
        // Pairs from the published CIEDE2000 test data (Sharma, Wu and Dalal, 2005).
        let cases: [([f64; 3], [f64; 3], f64); 6] = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, 2.5, 0.0], [50.0, 0.0, -2.5], 4.3065),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
            ([22.7233, 20.0904, -46.694], [23.0331, 14.973, -42.5619], 2.0373),
        ];
        for (a, b, want) in cases {
            let d = delta_e00(a, b);
            assert!((d - want).abs() < 1e-3, "{a:?} {b:?}: {d} != {want}");
            assert!((delta_e00(b, a) - want).abs() < 1e-3, "symmetric");
        }
        assert_eq!(delta_e00([50.0, 10.0, 10.0], [50.0, 10.0, 10.0]), 0.0);
    }

    /// Applies a `colorGrade` object to an image the way the renderer does (wheels and saturation only).
    fn graded(src: &Rgba, grade: &Value) -> Rgba {
        let mut f = Map::new();
        f.insert("colorGrade".into(), grade.clone());
        let mut out = src.clone();
        let Some(g) = ove_render::color::ColorGrade::from_fields(&f, &|_| None) else { return out };
        for p in out.data.chunks_exact_mut(4) {
            let c = g.eval([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]);
            for k in 0..3 {
                p[k] = (c[k] * 255.0).round() as u8;
            }
        }
        out
    }

    /// A busy test picture: gradients in every channel plus a skin-tone patch.
    fn scene() -> Rgba {
        img(160, 90, |x, y| {
            if (60..100).contains(&x) && (30..60).contains(&y) {
                return [210, 160, 130];
            }
            [(x * 255 / 159) as u8, (y * 255 / 89) as u8, ((x + y) * 255 / 248) as u8]
        })
    }

    #[test]
    fn suggestion_converges_on_a_warm_shift() {
        let reference = scene();
        let warm = img(160, 90, |x, y| {
            let [r, g, b] = [reference.px(x, y)[0], reference.px(x, y)[1], reference.px(x, y)[2]];
            [
                (r as f64 * 1.08 + 18.0).min(255.0) as u8,
                (g as f64 * 1.02 + 4.0).min(255.0) as u8,
                (b as f64 * 0.85).min(255.0) as u8,
            ]
        });
        let target = color_stats(&reference, false);
        let mut grade = Value::Null;
        let start = gap(&color_stats(&warm, false), &target).mean;
        assert!(start > 4.0, "the shift is visible: {start}");
        let mut last = start;
        for _ in 0..3 {
            let shot = if grade.is_null() { warm.clone() } else { graded(&warm, &grade) };
            grade = suggest_grade(&reference, &shot, (!grade.is_null()).then_some(&grade), 1.0);
            last = gap(&color_stats(&graded(&warm, &grade), false), &target).mean;
        }
        assert!(last < 2.0, "after three rounds: {last} (from {start}); grade {grade}");
        assert!(grade.get("match").is_none());
    }

    #[test]
    fn suggestion_steps_are_capped_and_keep_other_grade_fields() {
        let reference = img(32, 32, |x, _| [(x * 8) as u8; 3]);
        let dark = img(32, 32, |x, _| [(x * 2) as u8; 3]);
        let current = json!({"lut": {"preset": "film"}, "wheels": {"lift": {"r": 0.1, "g": 0, "b": 0, "y": 0}}});
        let g = suggest_grade(&reference, &dark, Some(&current), 1.0);
        assert_eq!(g["lut"]["preset"], "film");
        assert_eq!(g["wheels"]["lift"]["r"], 0.1);
        for c in ["r", "g", "b"] {
            let v = g["wheels"]["gain"][c].as_f64().unwrap();
            assert!(v <= MAX_GAIN_STEP + 1e-9, "gain step capped: {v}");
        }
        // Strength 0 changes nothing.
        let none = suggest_grade(&reference, &dark, None, 0.0);
        assert_eq!(none["wheels"]["gain"]["r"], 0.0);
        assert_eq!(none["saturation"], 1.0);
    }
}
