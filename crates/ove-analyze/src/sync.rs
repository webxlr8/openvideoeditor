//! Audio sync (add-audio-intelligence design D2–D4): where a target recording sits against a reference recording,
//! to below a millisecond, and how fast its clock drifts.
//!
//! 1. **Coarse**: a cross-correlation over every offset of the cached onset envelopes (10 ms hops), computed with
//!    one FFT and normalised by running energy sums over the overlap, so partial overlaps compare fairly. A match
//!    needs [`SyncParams::min_overlap_s`] of overlap and is refused when the best peak is not clearly above the
//!    second-best one (repeated music gives several equal peaks).
//! 2. **Fine**: a generalised cross-correlation with phase transform (GCC-PHAT) on band-limited (300 Hz – 4 kHz)
//!    raw audio within ±20 ms of the coarse peak, with parabolic interpolation, for sub-millisecond offsets.
//! 3. **Drift**: for overlaps longer than 150 s, the offset is measured in windows every 30 s and fitted with a
//!    Theil–Sen line, which ignores outlier windows. The slope is the target clock's drift.
//!
//! Times are source seconds. The *lag* at reference time `t` is `t − u`, where `u` is the target time whose sound
//! is heard at `t`. The result's `offset` is the reference time at which the target's time 0 sits, and
//! `drift_ppm` is how much faster the target clock runs: `u = (1 + drift) · (t − offset)`.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use serde::Serialize;

use crate::{Error, Result};

/// Sample rate of raw audio reads.
pub const RATE: u32 = 48_000;
/// Seconds per onset-envelope value (the analysis cache's hop).
pub const ENVELOPE_HOP_S: f64 = crate::analysis::ONSET_HOP as f64 / RATE as f64;

/// Mono raw audio at [`RATE`] for the fine step.
pub trait Source {
    /// Fills `out` with mono samples starting at sample `start` (may be negative); zero outside the recording.
    fn read(&mut self, start: i64, out: &mut [f32]) -> Result<()>;
}

/// Tuning of one sync run.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncParams {
    /// Shortest overlap accepted, seconds.
    pub min_overlap_s: f64,
    /// Lowest confidence (0–1) accepted.
    pub min_confidence: f64,
    /// The best peak must be at least this many times the second-best peak.
    pub min_peak_ratio: f64,
    /// Largest offset searched, seconds (`None`: every offset).
    pub window_s: Option<f64>,
    /// Measure drift for long overlaps.
    pub drift: bool,
    /// The target may be retimed to correct drift (audio-only targets; video items keep their picture timing).
    pub retime: bool,
}

impl Default for SyncParams {
    fn default() -> Self {
        SyncParams {
            min_overlap_s: 3.0,
            min_confidence: 0.2,
            min_peak_ratio: 1.3,
            window_s: None,
            drift: true,
            retime: true,
        }
    }
}

/// Overlaps at least this long get a drift measurement.
pub const DRIFT_MIN_OVERLAP_S: f64 = 150.0;
/// Seconds between drift windows.
pub const DRIFT_SPACING_S: f64 = 30.0;
/// Drift (absolute, ppm) applied as a correction: smaller values are clock jitter, larger ones are not clock drift.
pub const DRIFT_RANGE_PPM: (f64, f64) = (20.0, 500.0);
/// Fine search half-width, seconds.
const FINE_SEARCH_S: f64 = 0.020;
/// Raw-audio window of the head measurement, samples.
const FINE_WINDOW: usize = 2 * RATE as usize;
/// Raw-audio window of each drift measurement, samples.
const DRIFT_WINDOW: usize = RATE as usize;
/// Lowest normalised GCC-PHAT peak that counts as a measurement.
const PHAT_MIN_PEAK: f32 = 0.05;
/// Peaks closer than this to the best one are the same peak when looking for the second-best, seconds.
const PEAK_EXCLUSION_S: f64 = 0.5;
/// Envelope segment matched against the reference when the target is long, seconds.
const PROBE_SEGMENT_S: f64 = 20.0;
/// Probe segments taken from a long target, one per equal part.
const PROBE_SEGMENTS: usize = 5;

/// The coarse envelope match.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coarse {
    /// Lag in seconds (reference time minus target time).
    pub lag_s: f64,
    /// Normalised correlation at the peak, overlap-weighted (0–1).
    pub confidence: f64,
    /// Best peak over the second-best peak.
    pub peak_ratio: f64,
    /// Overlap of the whole recordings at this lag, seconds.
    pub overlap_s: f64,
}

/// Drift measurement.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drift {
    /// How much faster the target clock runs, parts per million.
    pub ppm: f64,
    /// Root-mean-square distance of the windows from the fitted line, milliseconds.
    pub residual_ms: f64,
    /// Windows measured and windows used by the fit.
    pub windows: usize,
    pub used: usize,
    /// True when the drift is in [`DRIFT_RANGE_PPM`] and is corrected.
    pub applied: bool,
    /// Why the drift is not corrected, when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Where the target sits against the reference.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    /// Reference time (s) at which the target's time 0 sits.
    pub offset_s: f64,
    /// Drift corrected by the placement, parts per million (0 when not applied).
    pub speed_ppm: f64,
    pub coarse: Coarse,
    /// GCC-PHAT peak of the head measurement (0–1), when the fine step ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fine_peak: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<Drift>,
}

/// Why a target is not matched.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub reason: String,
    /// The best coarse candidate, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coarse: Option<Coarse>,
}

/// The onset envelope as floats.
pub fn envelope(onset: &[u8]) -> Vec<f32> {
    onset.iter().map(|v| *v as f32).collect()
}

// -------------------------------------------------------------------------------------------------------------
// FFT helpers
// -------------------------------------------------------------------------------------------------------------

struct Ffts {
    planner: FftPlanner<f32>,
}

impl Ffts {
    fn new() -> Ffts {
        Ffts { planner: FftPlanner::new() }
    }

    fn pair(&mut self, n: usize) -> (Arc<dyn Fft<f32>>, Arc<dyn Fft<f32>>) {
        (self.planner.plan_fft_forward(n), self.planner.plan_fft_inverse(n))
    }

    /// Linear cross-correlation `c[L] = Σ a[i + L] · b[i]` for `L` in `-(b.len() − 1) ..= a.len() − 1`, returned
    /// with index `L + b.len() − 1`.
    fn xcorr(&mut self, a: &[f32], b: &[f32]) -> Vec<f32> {
        let n = (a.len() + b.len()).next_power_of_two();
        let (fwd, inv) = self.pair(n);
        let mut fa: Vec<Complex32> = a.iter().map(|v| Complex32::new(*v, 0.0)).collect();
        fa.resize(n, Complex32::default());
        let mut fb: Vec<Complex32> = b.iter().map(|v| Complex32::new(*v, 0.0)).collect();
        fb.resize(n, Complex32::default());
        fwd.process(&mut fa);
        fwd.process(&mut fb);
        for (x, y) in fa.iter_mut().zip(&fb) {
            *x *= y.conj();
        }
        inv.process(&mut fa);
        let scale = 1.0 / n as f32;
        let nb = b.len();
        (0..a.len() + nb - 1)
            .map(|k| {
                let lag = k as i64 - (nb as i64 - 1);
                let idx = if lag >= 0 { lag as usize } else { (n as i64 + lag) as usize };
                fa[idx].re * scale
            })
            .collect()
    }
}

fn prefix_sq(v: &[f32]) -> Vec<f64> {
    let mut out = Vec::with_capacity(v.len() + 1);
    out.push(0.0);
    let mut acc = 0.0f64;
    for x in v {
        acc += (*x as f64) * (*x as f64);
        out.push(acc);
    }
    out
}

fn zero_mean(v: &[f32]) -> Vec<f32> {
    let mean = if v.is_empty() { 0.0 } else { v.iter().map(|x| *x as f64).sum::<f64>() / v.len() as f64 };
    v.iter().map(|x| (*x as f64 - mean) as f32).collect()
}

/// Peak position refined by a parabola through the peak and its neighbours (−0.5..0.5).
fn parabolic(ym: f64, y0: f64, yp: f64) -> f64 {
    let d = ym - 2.0 * y0 + yp;
    if d.abs() < 1e-12 { 0.0 } else { (0.5 * (ym - yp) / d).clamp(-0.5, 0.5) }
}

// -------------------------------------------------------------------------------------------------------------
// Coarse search
// -------------------------------------------------------------------------------------------------------------

/// Normalised correlation scores of `b` against `a` over every lag (index as in [`Ffts::xcorr`]); lags whose overlap
/// is shorter than `min_overlap` hops score 0. Overlaps shorter than `full` hops are weighted down by
/// `sqrt(overlap / full)`, so a short chance match does not beat a long true one.
fn scores(ffts: &mut Ffts, a: &[f32], b: &[f32], min_overlap: usize, full: usize) -> Vec<f32> {
    let (za, zb) = (zero_mean(a), zero_mean(b));
    let c = ffts.xcorr(&za, &zb);
    let (pa, pb) = (prefix_sq(&za), prefix_sq(&zb));
    let (na, nb) = (a.len() as i64, b.len() as i64);
    c.iter()
        .enumerate()
        .map(|(k, v)| {
            let lag = k as i64 - (nb - 1);
            let a0 = lag.max(0);
            let a1 = na.min(nb + lag);
            let ov = a1 - a0;
            if ov < min_overlap as i64 || ov <= 0 {
                return 0.0;
            }
            let b0 = a0 - lag;
            let ea = pa[a1 as usize] - pa[a0 as usize];
            let eb = pb[(b0 + ov) as usize] - pb[b0 as usize];
            if ea <= 1e-9 || eb <= 1e-9 {
                return 0.0;
            }
            let w = ((ov as f64) / full.max(1) as f64).min(1.0).sqrt();
            ((*v as f64) / (ea * eb).sqrt() * w) as f32
        })
        .collect()
}

/// The best and second-best peaks of `s` within `allowed` lags.
fn best_peaks(s: &[f32], nb: usize, exclusion: usize, allowed: impl Fn(i64) -> bool) -> Option<(usize, f32, f32)> {
    let lag = |k: usize| k as i64 - (nb as i64 - 1);
    let best = (0..s.len()).filter(|k| allowed(lag(*k))).max_by(|x, y| s[*x].total_cmp(&s[*y]))?;
    if s[best] <= 0.0 {
        return None;
    }
    let second =
        (0..s.len()).filter(|k| allowed(lag(*k)) && k.abs_diff(best) > exclusion).map(|k| s[k]).fold(0.0f32, f32::max);
    Some((best, s[best], second))
}

/// Coarse match of the target envelope `tgt` against the reference envelope `reference` (hops of
/// [`ENVELOPE_HOP_S`]). A long target is matched by its most active 20 s in each fifth (so clock drift does not
/// smear the match), and the clearest of those matches wins.
pub fn coarse(reference: &[f32], tgt: &[f32], p: &SyncParams) -> std::result::Result<Coarse, Refusal> {
    let hop = ENVELOPE_HOP_S;
    let min_overlap = (p.min_overlap_s / hop).ceil() as usize;
    let full = (30.0 / hop) as usize;
    let seg_len = (PROBE_SEGMENT_S / hop) as usize;
    let segments: Vec<usize> = if tgt.len() <= 2 * seg_len {
        vec![0]
    } else {
        // Most active `seg_len` hops within each part of the target.
        let pre = prefix_sq(tgt);
        let part = tgt.len() / PROBE_SEGMENTS;
        let last = tgt.len() - seg_len;
        let mut starts: Vec<usize> = (0..PROBE_SEGMENTS)
            .filter_map(|i| {
                let (lo, hi) = ((i * part).min(last), ((i + 1) * part).min(last));
                (lo..=hi).step_by(50).max_by(|x, y| {
                    let ex = pre[(*x + seg_len).min(tgt.len())] - pre[*x];
                    let ey = pre[(*y + seg_len).min(tgt.len())] - pre[*y];
                    ex.total_cmp(&ey)
                })
            })
            .collect();
        starts.dedup();
        starts
    };
    let mut ffts = Ffts::new();
    let exclusion = (PEAK_EXCLUSION_S / hop) as usize;
    let mut best: Option<Coarse> = None;
    for start in segments {
        let seg = if start == 0 && tgt.len() <= 2 * seg_len { tgt } else { &tgt[start..start + seg_len] };
        let s = scores(&mut ffts, reference, seg, min_overlap.min(seg.len()), full.min(seg.len()));
        let window = p.window_s.map(|w| (w / hop).ceil() as i64);
        let shift = start as i64; // segment lag → whole-target lag
        let Some((k, top, second)) =
            best_peaks(&s, seg.len(), exclusion, |lag| window.is_none_or(|w| (lag - shift).abs() <= w))
        else {
            continue;
        };
        let frac =
            if k > 0 && k + 1 < s.len() { parabolic(s[k - 1] as f64, s[k] as f64, s[k + 1] as f64) } else { 0.0 };
        let lag_hops = k as f64 - (seg.len() as f64 - 1.0) + frac - shift as f64;
        let lag_s = lag_hops * hop;
        let overlap_s = (reference.len() as f64 * hop).min(lag_s + tgt.len() as f64 * hop) - lag_s.max(0.0);
        let c = Coarse {
            lag_s,
            confidence: (top as f64).clamp(0.0, 1.0),
            peak_ratio: if second > 1e-6 { (top / second) as f64 } else { 99.0 },
            overlap_s: overlap_s.max(0.0),
        };
        let better = match &best {
            None => true,
            Some(b) => (c.confidence * c.peak_ratio.min(3.0)) > (b.confidence * b.peak_ratio.min(3.0)),
        };
        if better {
            best = Some(c);
        }
    }
    let Some(c) = best else {
        return Err(Refusal { reason: "no overlap of at least the minimum length".into(), coarse: None });
    };
    if c.overlap_s < p.min_overlap_s {
        return Err(Refusal {
            reason: format!("the recordings overlap by {:.1} s, less than {:.1} s", c.overlap_s, p.min_overlap_s),
            coarse: Some(c),
        });
    }
    if c.confidence < p.min_confidence {
        return Err(Refusal {
            reason: format!(
                "confidence {:.2} is below {:.2}: the sound does not match",
                c.confidence, p.min_confidence
            ),
            coarse: Some(c),
        });
    }
    if c.peak_ratio < p.min_peak_ratio {
        return Err(Refusal {
            reason: format!(
                "ambiguous: the best match is only {:.2}× the next one (repeated sound); refused rather than guessed",
                c.peak_ratio
            ),
            coarse: Some(c),
        });
    }
    Ok(c)
}

// -------------------------------------------------------------------------------------------------------------
// Fine search
// -------------------------------------------------------------------------------------------------------------

/// GCC-PHAT between two equal-length windows: the residual lag `d` (samples, `x[n] ≈ y[n − d]`) within
/// `±search` samples and its normalised peak.
fn gcc_phat(ffts: &mut Ffts, x: &[f32], y: &[f32], search: usize) -> Option<(f64, f32)> {
    let w = x.len().min(y.len());
    if w < 64 {
        return None;
    }
    let n = (w + 2 * search).next_power_of_two();
    let (fwd, inv) = ffts.pair(n);
    let hann = |i: usize| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (w - 1) as f32).cos();
    let mut fx: Vec<Complex32> =
        (0..n).map(|i| Complex32::new(if i < w { x[i] * hann(i) } else { 0.0 }, 0.0)).collect();
    let mut fy: Vec<Complex32> =
        (0..n).map(|i| Complex32::new(if i < w { y[i] * hann(i) } else { 0.0 }, 0.0)).collect();
    fwd.process(&mut fx);
    fwd.process(&mut fy);
    let bin_hz = RATE as f32 / n as f32;
    let (lo, hi) = ((300.0 / bin_hz) as usize, (4_000.0 / bin_hz).ceil() as usize);
    let mut used = 0usize;
    for (k, (a, b)) in fx.iter_mut().zip(&fy).enumerate() {
        let f = k.min(n - k);
        let g = *a * b.conj();
        let m = g.norm();
        if f >= lo && f <= hi && m > 1e-20 {
            *a = g / m;
            used += 1;
        } else {
            *a = Complex32::default();
        }
    }
    if used == 0 {
        return None;
    }
    inv.process(&mut fx);
    let at = |d: i64| -> f32 { fx[if d >= 0 { d as usize } else { (n as i64 + d) as usize }].re / used as f32 };
    let s = search as i64;
    let d = (-s..=s).max_by(|a, b| at(*a).total_cmp(&at(*b)))?;
    let peak = at(d);
    let frac = if d > -s && d < s { parabolic(at(d - 1) as f64, peak as f64, at(d + 1) as f64) } else { 0.0 };
    Some((d as f64 + frac, peak))
}

/// The lag at reference sample `at` refined from `lag_s` by GCC-PHAT on `window` samples, searching `±search_s`.
fn refine(
    ffts: &mut Ffts,
    reference: &mut dyn Source,
    tgt: &mut dyn Source,
    at: i64,
    lag_s: f64,
    window: usize,
    search_s: f64,
) -> Result<Option<(f64, f32)>> {
    let lag = (lag_s * RATE as f64).round() as i64;
    let mut x = vec![0.0f32; window];
    let mut y = vec![0.0f32; window];
    reference.read(at, &mut x)?;
    tgt.read(at - lag, &mut y)?;
    let search = (search_s * RATE as f64).ceil() as usize;
    Ok(gcc_phat(ffts, &x, &y, search)
        .filter(|(_, p)| *p >= PHAT_MIN_PEAK)
        .map(|(d, p)| ((lag as f64 + d) / RATE as f64, p)))
}

/// The most active window of `len_s` seconds within reference seconds `[lo, hi)`, by envelope energy.
fn active_window(env: &[f32], lo: f64, hi: f64, len_s: f64) -> f64 {
    let hop = ENVELOPE_HOP_S;
    let (a, b) = ((lo / hop).max(0.0) as usize, ((hi / hop) as usize).min(env.len()));
    let len = ((len_s / hop) as usize).max(1);
    if b <= a + len {
        return lo;
    }
    let pre = prefix_sq(env);
    let best = (a..=b - len).step_by(5).max_by(|x, y| (pre[x + len] - pre[*x]).total_cmp(&(pre[y + len] - pre[*y])));
    best.map_or(lo, |k| k as f64 * hop)
}

// -------------------------------------------------------------------------------------------------------------
// Drift
// -------------------------------------------------------------------------------------------------------------

/// Theil–Sen line through `(x, y)`: the median of pairwise slopes and the median intercept.
pub fn theil_sen(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.len() < 2 {
        return None;
    }
    let mut slopes = Vec::with_capacity(points.len() * (points.len() - 1) / 2);
    for (i, a) in points.iter().enumerate() {
        for b in &points[i + 1..] {
            if (b.0 - a.0).abs() > 1e-9 {
                slopes.push((b.1 - a.1) / (b.0 - a.0));
            }
        }
    }
    let slope = median(&mut slopes)?;
    let mut icpt: Vec<f64> = points.iter().map(|(x, y)| y - slope * x).collect();
    Some((slope, median(&mut icpt)?))
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) })
}

/// Local envelope match of a 20 s reference segment at `t` against the target near `lag_s`, within `±search_s`.
fn local_coarse(ffts: &mut Ffts, reference: &[f32], tgt: &[f32], t: f64, lag_s: f64, search_s: f64) -> Option<f64> {
    let hop = ENVELOPE_HOP_S;
    let seg = (20.0 / hop) as usize;
    let a0 = (t / hop) as usize;
    let a1 = (a0 + seg).min(reference.len());
    if a1 <= a0 + 100 {
        return None;
    }
    let search = (search_s / hop).ceil() as i64;
    // Target hops [b0, b1) cover the segment at every lag within the search.
    let b0 = (((t - lag_s) / hop).floor() as i64 - search).max(0);
    let b1 = ((((t - lag_s) / hop).ceil() as i64) + (a1 - a0) as i64 + search).min(tgt.len() as i64);
    if b1 <= b0 + 100 {
        return None;
    }
    let (a, b) = (&reference[a0..a1], &tgt[b0 as usize..b1 as usize]);
    // Lag of the segment: reference hop a0 + i matches target hop b0 + j → lag = (a0 − b0) + (i − j).
    let s = scores(ffts, a, b, (a.len() / 2).min(b.len()), a.len());
    let base = a0 as i64 - b0;
    let centre = (lag_s / hop).round() as i64;
    let (k, top, _) = best_peaks(&s, b.len(), 0, |l| ((base + l) - centre).abs() <= search)?;
    (top > 0.1).then(|| (base + k as i64 - (b.len() as i64 - 1)) as f64 * hop)
}

/// Measures the drift across the overlap: windows every [`DRIFT_SPACING_S`], each matched by envelope near the
/// coarse lag (within ±2,500 ppm of travel) and refined by GCC-PHAT. Returns the fit `lag = a + b·t` and the drift.
fn measure_drift(
    ffts: &mut Ffts,
    env_ref: &[f32],
    env_tgt: &[f32],
    reference: &mut dyn Source,
    tgt: &mut dyn Source,
    c: &Coarse,
    (head_s, tail_s): (f64, f64),
) -> Result<Option<(f64, f64, Drift)>> {
    let mid = 0.5 * (head_s + tail_s);
    let mut points = Vec::new();
    let mut windows = 0usize;
    let mut t = head_s;
    while t + 20.0 <= tail_s + 1e-9 {
        windows += 1;
        let search = 0.1 + 2.5e-3 * (t + 10.0 - mid).abs();
        if let Some(local) = local_coarse(ffts, env_ref, env_tgt, t, c.lag_s, search) {
            let at = active_window(env_ref, t, t + 20.0, DRIFT_WINDOW as f64 / RATE as f64);
            let at_sample = (at * RATE as f64).round() as i64;
            let w = DRIFT_WINDOW as f64 / RATE as f64;
            if let Some((lag, _)) = refine(ffts, reference, tgt, at_sample, local, DRIFT_WINDOW, FINE_SEARCH_S)? {
                // The lag measured over the window belongs to its middle.
                points.push((at + 0.5 * w, lag));
            }
        }
        t += DRIFT_SPACING_S;
    }
    let Some((b, a)) = theil_sen(&points) else { return Ok(None) };
    let res: Vec<f64> = points.iter().map(|(x, y)| (y - (a + b * x)) * 1000.0).collect();
    // Windows more than 5 ms off the line are outliers (a mismatched window); the residual is over the rest.
    let inliers: Vec<f64> = res.iter().copied().filter(|r| r.abs() <= 5.0).collect();
    let rms = if inliers.is_empty() {
        f64::NAN
    } else {
        (inliers.iter().map(|r| r * r).sum::<f64>() / inliers.len() as f64).sqrt()
    };
    let ppm = -b * 1e6;
    let (lo, hi) = DRIFT_RANGE_PPM;
    let (applied, note) = if inliers.len() < 3 {
        (false, Some(format!("only {} of {windows} windows matched; drift not measured", inliers.len())))
    } else if ppm.abs() < lo {
        (false, Some(format!("{ppm:.1} ppm is below {lo} ppm: no correction needed")))
    } else if ppm.abs() > hi {
        (false, Some(format!("{ppm:.0} ppm is out of range (over {hi} ppm): reported, not applied")))
    } else {
        (true, None)
    };
    let drift = Drift {
        ppm: (ppm * 100.0).round() / 100.0,
        residual_ms: (rms * 1000.0).round() / 1000.0,
        windows,
        used: inliers.len(),
        applied,
        note,
    };
    Ok(Some((a, b, drift)))
}

// -------------------------------------------------------------------------------------------------------------
// The whole match
// -------------------------------------------------------------------------------------------------------------

/// Aligns a target recording to a reference one: coarse by envelope, fine by GCC-PHAT, then drift.
pub fn align(
    env_ref: &[f32],
    reference: &mut dyn Source,
    env_tgt: &[f32],
    tgt: &mut dyn Source,
    p: &SyncParams,
) -> Result<std::result::Result<Match, Refusal>> {
    let c = match coarse(env_ref, env_tgt, p) {
        Ok(c) => c,
        Err(r) => return Ok(Err(r)),
    };
    let mut ffts = Ffts::new();
    let hop = ENVELOPE_HOP_S;
    let head = c.lag_s.max(0.0);
    let tail = (env_ref.len() as f64 * hop).min(c.lag_s + env_tgt.len() as f64 * hop);
    // Head measurement: the most active two seconds of the overlap.
    let fw = FINE_WINDOW as f64 / RATE as f64;
    let at = active_window(env_ref, head, tail, fw);
    let at_sample = (at * RATE as f64).round() as i64;
    // The coarse lag is within a hop or two; search the fine window around it.
    let fine = refine(&mut ffts, reference, tgt, at_sample, c.lag_s, FINE_WINDOW, FINE_SEARCH_S)?;
    let (mut lag, fine_peak) = match fine {
        Some((l, p)) => (l, Some(p as f64)),
        None => (c.lag_s, None),
    };
    let mut drift = None;
    let mut speed_ppm = 0.0;
    let measured = if p.drift && tail - head >= DRIFT_MIN_OVERLAP_S {
        measure_drift(&mut ffts, env_ref, env_tgt, reference, tgt, &c, (head, tail))?
    } else {
        None
    };
    {
        if let Some((a, b, mut d)) = measured {
            if d.applied && !p.retime {
                d.applied = false;
                d.note = Some("video items keep their speed; the offset is fitted at the middle of the overlap".into());
            }
            if d.applied {
                // u = t − (a + b t) = (1 − b)(t − a / (1 − b)).
                speed_ppm = -b * 1e6;
                lag = a / (1.0 - b);
            } else if d.used >= 3 {
                // Constant offset: the fitted line at the middle of the overlap.
                let m = 0.5 * (head + tail);
                lag = a + b * m;
            }
            drift = Some(d);
        }
    }
    Ok(Ok(Match { offset_s: lag, speed_ppm, coarse: c, fine_peak, drift }))
}

/// Reads mono audio from a media file for the fine step (the sum of all channels, resampled to [`RATE`]).
pub struct FileSource {
    path: std::path::PathBuf,
    dec: Option<ove_media::AudioDecoder>,
}

impl FileSource {
    pub fn new(path: &std::path::Path) -> FileSource {
        FileSource { path: path.to_owned(), dec: None }
    }
}

impl Source for FileSource {
    fn read(&mut self, start: i64, out: &mut [f32]) -> Result<()> {
        use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
        out.iter_mut().for_each(|v| *v = 0.0);
        let end = start + out.len() as i64;
        if end <= 0 {
            return Ok(());
        }
        if self.dec.is_none() {
            ove_media::init();
            self.dec = Some(AudioDecoder::open(
                &self.path,
                AudioOutputSpec { sample_rate: RATE, channels: 1, format: SampleFormat::F32 },
            )?);
        }
        let Some(dec) = self.dec.as_mut() else { return Err(Error::Invalid("no decoder".into())) };
        let from = start.max(0);
        dec.seek(from * 1_000_000 / RATE as i64)?;
        while let Some(chunk) = dec.next_chunk()? {
            let Samples::F32(s) = &chunk.samples else { continue };
            let first = (chunk.pts_us as i128 * RATE as i128 + 500_000).div_euclid(1_000_000) as i64;
            for (i, v) in s.iter().enumerate() {
                let n = first + i as i64;
                if n >= start && n < end {
                    out[(n - start) as usize] = *v;
                }
            }
            if first + s.len() as i64 >= end {
                break;
            }
        }
        Ok(())
    }
}

/// An in-memory recording (tests, and audio already decoded).
pub struct MemorySource<'a>(pub &'a [f32]);

impl Source for MemorySource<'_> {
    fn read(&mut self, start: i64, out: &mut [f32]) -> Result<()> {
        for (i, v) in out.iter_mut().enumerate() {
            let n = start + i as i64;
            *v = if n >= 0 { self.0.get(n as usize).copied().unwrap_or(0.0) } else { 0.0 };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theil_sen_ignores_outliers() {
        let mut pts: Vec<(f64, f64)> = (0..20).map(|i| (i as f64, 2.0 + 0.5 * i as f64)).collect();
        pts[3].1 = 100.0;
        pts[11].1 = -50.0;
        let (b, a) = theil_sen(&pts).unwrap();
        assert!((b - 0.5).abs() < 1e-9 && (a - 2.0).abs() < 1e-9, "{b} {a}");
    }

    #[test]
    fn xcorr_lag_sign() {
        // a is b delayed by 3: a[i + 3] = b[i].
        let b: Vec<f32> = (0..32).map(|i| ((i * 7919) % 13) as f32 - 6.0).collect();
        let mut a = vec![0.0; 3];
        a.extend_from_slice(&b);
        let c = Ffts::new().xcorr(&a, &b);
        let k = (0..c.len()).max_by(|x, y| c[*x].total_cmp(&c[*y])).unwrap();
        assert_eq!(k as i64 - (b.len() as i64 - 1), 3);
    }

    #[test]
    fn gcc_phat_finds_a_fractional_delay() {
        // Band-limited noise; y is x advanced by 7.4 samples (x[n] = y[n − 7.4]).
        let mut s = 1u64;
        let noise: Vec<f32> = (0..20_000)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s % 2001) as f32 / 1000.0 - 1.0
            })
            .collect();
        let interp = |t: f64| -> f32 {
            // Windowed-sinc interpolation of `noise` at fractional index t.
            let mut acc = 0.0f64;
            let c = t.floor() as i64;
            for k in c - 32..=c + 32 {
                if k < 0 || k as usize >= noise.len() {
                    continue;
                }
                let x = t - k as f64;
                let sinc =
                    if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
                let w = 0.5 + 0.5 * (std::f64::consts::PI * x / 33.0).cos();
                acc += noise[k as usize] as f64 * sinc * w;
            }
            acc as f32
        };
        let x: Vec<f32> = (0..8192).map(|n| interp(5000.0 + n as f64)).collect();
        let y: Vec<f32> = (0..8192).map(|n| interp(5000.0 + n as f64 + 7.4)).collect();
        let (d, peak) = gcc_phat(&mut Ffts::new(), &x, &y, 50).unwrap();
        assert!((d - 7.4).abs() < 0.1, "{d}");
        assert!(peak > 0.3, "{peak}");
    }
}
