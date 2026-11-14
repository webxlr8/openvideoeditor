//! Audio analysis in one streaming pass over the decoded file (48 kHz float, the source's channel count): peak and
//! RMS levels, waveform images, silence ranges, and loudness per ITU-R BS.1770-4 / EBU R128 (integrated loudness
//! with absolute and relative gating, loudness range per EBU Tech 3342, sample peak and a 4× oversampled true-peak
//! estimate).
//!
//! Levels in dB are floored at [`FLOOR_DB`] (digital silence).

use std::path::Path;

use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
use serde::Serialize;

use crate::image::Image;
use crate::{Error, Result};

/// Lowest level reported, in dB.
pub const FLOOR_DB: f64 = -120.0;
/// Analysis sample rate.
pub const RATE: u32 = 48_000;
/// Level windows are 10 ms.
const WINDOW: usize = (RATE / 100) as usize;

fn db(x: f64) -> f64 {
    if x <= 0.0 { FLOOR_DB } else { (20.0 * x.log10()).max(FLOOR_DB) }
}

fn db_power(p: f64) -> f64 {
    if p <= 0.0 { FLOOR_DB } else { (10.0 * p.log10()).max(FLOOR_DB) }
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// One biquad section, transposed direct form II.
#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The two K-weighting stages (high shelf, then high pass) of BS.1770, designed for sample rate `fs`.
fn k_weighting(fs: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    // Stage 1: high shelf, +4 dB above ~1.7 kHz.
    let (f0, g, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    // Stage 2: high pass at ~38 Hz.
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], z: [0.0; 2] };
    [shelf, hp]
}

/// BS.1770 channel weights for the default FFmpeg layouts (LFE excluded, surrounds +1.5 dB).
fn channel_weights(ch: usize) -> Vec<f64> {
    match ch {
        6 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41],
        8 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 1.41, 1.41],
        n => vec![1.0; n],
    }
}

/// 4× oversampling interpolator for true peak: 12-tap Hann-windowed sinc per phase.
const TP_TAPS: usize = 12;

fn tp_kernel() -> [[f64; TP_TAPS]; 3] {
    use std::f64::consts::PI;
    let mut k = [[0.0; TP_TAPS]; 3];
    for (p, row) in k.iter_mut().enumerate() {
        let frac = (p + 1) as f64 / 4.0;
        for (j, v) in row.iter_mut().enumerate() {
            // Distance from the interpolated point (between taps 5 and 6) to tap j.
            let d = 5.0 + frac - j as f64;
            let sinc = if d.abs() < 1e-12 { 1.0 } else { (PI * d).sin() / (PI * d) };
            let w = if d.abs() < 6.5 { 0.5 * (1.0 + (PI * d / 6.5).cos()) } else { 0.0 };
            *v = sinc * w;
        }
    }
    k
}

/// Everything the public functions need, from one decode.
struct Pass {
    channels: usize,
    weights: Vec<f64>,
    filters: Vec<[Biquad; 2]>,
    /// K-weighted mean-square accumulators of the current 100 ms sub-block, per channel.
    sub_acc: Vec<f64>,
    sub_n: usize,
    /// Weighted channel sum of mean squares per complete 100 ms sub-block.
    subs: Vec<f64>,
    /// (peak, mean square) of each complete or final partial 10 ms window, over all channels.
    windows: Vec<(f32, f32)>,
    win_peak: f64,
    win_sq: f64,
    win_n: usize,
    sample_peak: f64,
    true_peak: f64,
    history: Vec<[f64; TP_TAPS]>,
    kernel: [[f64; TP_TAPS]; 3],
    frames: u64,
}

impl Pass {
    fn new(channels: usize) -> Pass {
        Pass {
            channels,
            weights: channel_weights(channels),
            filters: vec![k_weighting(RATE as f64); channels],
            sub_acc: vec![0.0; channels],
            sub_n: 0,
            subs: Vec::new(),
            windows: Vec::new(),
            win_peak: 0.0,
            win_sq: 0.0,
            win_n: 0,
            sample_peak: 0.0,
            true_peak: 0.0,
            history: vec![[0.0; TP_TAPS]; channels],
            kernel: tp_kernel(),
            frames: 0,
        }
    }

    fn push_frame(&mut self, frame: &[f32]) {
        let sub_len = (RATE / 10) as usize;
        for (c, &s) in frame.iter().enumerate().take(self.channels) {
            let x = s as f64;
            let a = x.abs();
            self.win_peak = self.win_peak.max(a);
            self.win_sq += x * x;
            self.sample_peak = self.sample_peak.max(a);
            // True peak: shift the history and interpolate three points between the two middle taps.
            let h = &mut self.history[c];
            h.copy_within(1.., 0);
            h[TP_TAPS - 1] = x;
            for phase in &self.kernel {
                let y: f64 = phase.iter().zip(h.iter()).map(|(k, v)| k * v).sum();
                self.true_peak = self.true_peak.max(y.abs());
            }
            let [s1, s2] = &mut self.filters[c];
            let z = s2.run(s1.run(x));
            self.sub_acc[c] += z * z;
        }
        self.frames += 1;
        self.win_n += 1;
        if self.win_n == WINDOW {
            self.close_window();
        }
        self.sub_n += 1;
        if self.sub_n == sub_len {
            let e = self.sub_acc.iter().zip(&self.weights).map(|(acc, w)| w * acc / sub_len as f64).sum();
            self.subs.push(e);
            self.sub_acc.iter_mut().for_each(|v| *v = 0.0);
            self.sub_n = 0;
        }
    }

    fn close_window(&mut self) {
        if self.win_n > 0 {
            let ms = self.win_sq / (self.win_n * self.channels) as f64;
            self.windows.push((self.win_peak as f32, ms as f32));
        }
        self.win_peak = 0.0;
        self.win_sq = 0.0;
        self.win_n = 0;
    }

    fn duration(&self) -> f64 {
        self.frames as f64 / RATE as f64
    }
}

/// Decodes a file's audio and runs the analysis pass.
fn analyse(path: &Path) -> Result<Pass> {
    ove_media::init();
    let info = ove_media::probe(path)?;
    let src = info.audio.as_ref().ok_or_else(|| Error::NoAudio(path.display().to_string()))?;
    let channels = src.channels.clamp(1, 8) as u16;
    let mut dec = AudioDecoder::open(path, AudioOutputSpec { sample_rate: RATE, channels, format: SampleFormat::F32 })?;
    let mut pass = Pass::new(channels as usize);
    while let Some(chunk) = dec.next_chunk()? {
        let Samples::F32(s) = &chunk.samples else { continue };
        for frame in s.chunks_exact(channels as usize) {
            pass.push_frame(frame);
        }
    }
    pass.close_window();
    Ok(pass)
}

/// Peak and RMS level per bucket, dBFS.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Levels {
    pub peaks_db: Vec<f32>,
    pub rms_db: Vec<f32>,
    /// Seconds each bucket covers.
    pub bucket_seconds: f64,
    pub duration: f64,
}

fn buckets(pass: &Pass, points: usize) -> Levels {
    let points = points.max(1);
    let n = pass.windows.len();
    let mut peaks = Vec::with_capacity(points);
    let mut rms = Vec::with_capacity(points);
    for b in 0..points {
        let (lo, hi) = (b * n / points, ((b + 1) * n / points).max(b * n / points + 1).min(n));
        let ws = pass.windows.get(lo..hi).unwrap_or(&[]);
        let peak = ws.iter().map(|w| w.0 as f64).fold(0.0, f64::max);
        let ms = if ws.is_empty() { 0.0 } else { ws.iter().map(|w| w.1 as f64).sum::<f64>() / ws.len() as f64 };
        peaks.push(r2(db(peak)) as f32);
        rms.push(r2(db_power(ms)) as f32);
    }
    Levels { peaks_db: peaks, rms_db: rms, bucket_seconds: pass.duration() / points as f64, duration: pass.duration() }
}

/// Peak and RMS levels of a file's audio in `points` equal buckets.
pub fn levels(path: &Path, points: usize) -> Result<Levels> {
    Ok(buckets(&analyse(path)?, points))
}

/// A waveform picture: one peak bar per column, mirrored around the centre line.
pub fn waveform_png(path: &Path, width: u32, height: u32) -> Result<Image> {
    let (width, height) = (width.max(1), height.max(2));
    let lv = buckets(&analyse(path)?, width as usize);
    let mut img = Image::filled(width, height, [20, 20, 24, 255]);
    let mid = height as f64 / 2.0;
    for (x, p) in lv.peaks_db.iter().enumerate() {
        let amp = if *p as f64 <= FLOOR_DB { 0.0 } else { 10f64.powf(*p as f64 / 20.0).min(1.0) };
        let half = (amp * mid).max(0.5);
        let (y0, y1) = ((mid - half).floor().max(0.0) as u32, ((mid + half).ceil() as u32).min(height));
        for y in y0..y1 {
            let i = ((y * width + x as u32) * 4) as usize;
            img.rgba[i..i + 4].copy_from_slice(&[110, 160, 255, 255]);
        }
    }
    Ok(img)
}

/// A time range in seconds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

/// Silent ranges: 10 ms windows whose RMS is below `threshold_db` dBFS, joined into runs at least `min_seconds` long.
pub fn silence(path: &Path, threshold_db: f64, min_seconds: f64) -> Result<Vec<Range>> {
    let pass = analyse(path)?;
    let w = WINDOW as f64 / RATE as f64;
    let total = pass.duration();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let flush = |s: usize, e: usize, out: &mut Vec<Range>| {
        let (a, b) = (s as f64 * w, (e as f64 * w).min(total));
        if b - a >= min_seconds - 1e-9 {
            out.push(Range { start: (a * 1000.0).round() / 1000.0, end: (b * 1000.0).round() / 1000.0 });
        }
    };
    for (i, win) in pass.windows.iter().enumerate() {
        let quiet = db_power(win.1 as f64) < threshold_db;
        match (quiet, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                flush(s, i, &mut out);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        flush(s, pass.windows.len(), &mut out);
    }
    Ok(out)
}

/// Loudness per BS.1770-4 / EBU R128.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loudness {
    /// Integrated loudness (LUFS); `None` when every block is below the −70 LUFS gate (silence or too short).
    pub integrated_lufs: Option<f64>,
    /// Loudness range (LU), EBU Tech 3342.
    pub loudness_range_lu: f64,
    /// Highest short-term (3 s) loudness, LUFS.
    pub max_short_term_lufs: Option<f64>,
    pub sample_peak_dbfs: f64,
    /// 4× oversampled estimate of the true peak.
    pub true_peak_dbtp: f64,
    pub channels: usize,
    pub duration: f64,
}

fn lufs(e: f64) -> f64 {
    -0.691 + 10.0 * e.log10()
}

/// Energies of windows of `len` sub-blocks stepped by one sub-block (100 ms).
fn window_energies(subs: &[f64], len: usize) -> Vec<f64> {
    if subs.len() < len {
        return vec![];
    }
    let mut out = Vec::with_capacity(subs.len() - len + 1);
    let mut acc: f64 = subs[..len].iter().sum();
    out.push(acc / len as f64);
    for i in len..subs.len() {
        acc += subs[i] - subs[i - len];
        out.push(acc.max(0.0) / len as f64);
    }
    out
}

/// Integrated loudness of momentary-block energies (absolute −70 LUFS gate, then relative −10 LU gate).
pub(crate) fn integrated(blocks: &[f64]) -> Option<f64> {
    let abs: Vec<f64> = blocks.iter().copied().filter(|e| *e > 0.0 && lufs(*e) > -70.0).collect();
    if abs.is_empty() {
        return None;
    }
    let gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let rel: Vec<f64> = abs.into_iter().filter(|e| lufs(*e) > gate).collect();
    (!rel.is_empty()).then(|| lufs(rel.iter().sum::<f64>() / rel.len() as f64))
}

/// Loudness range of short-term energies (absolute −70 LUFS, relative −20 LU gate; 95th − 10th percentile).
pub(crate) fn loudness_range(short: &[f64]) -> f64 {
    let abs: Vec<f64> = short.iter().copied().filter(|e| *e > 0.0 && lufs(*e) > -70.0).collect();
    if abs.is_empty() {
        return 0.0;
    }
    let gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 20.0;
    let mut l: Vec<f64> = abs.into_iter().map(lufs).filter(|v| *v > gate).collect();
    if l.len() < 2 {
        return 0.0;
    }
    l.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        let x = p * (l.len() - 1) as f64;
        let (i, f) = (x.floor() as usize, x.fract());
        l[i] + (l[(i + 1).min(l.len() - 1)] - l[i]) * f
    };
    pct(0.95) - pct(0.10)
}

/// Loudness of a file's audio.
pub fn loudness(path: &Path) -> Result<Loudness> {
    let pass = analyse(path)?;
    let blocks = window_energies(&pass.subs, 4);
    let short = window_energies(&pass.subs, 30);
    let max_short = short
        .iter()
        .copied()
        .filter(|e| *e > 0.0)
        .map(lufs)
        .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    Ok(Loudness {
        integrated_lufs: integrated(&blocks).map(r2),
        loudness_range_lu: r2(loudness_range(&short)),
        max_short_term_lufs: max_short.map(r2),
        sample_peak_dbfs: r2(db(pass.sample_peak)),
        true_peak_dbtp: r2(db(pass.true_peak.max(pass.sample_peak))),
        channels: pass.channels,
        duration: (pass.duration() * 1000.0).round() / 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(pass: &mut Pass, freq: f64, amp: f64, seconds: f64) {
        let n = (seconds * RATE as f64) as usize;
        for i in 0..n {
            let v = (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / RATE as f64).sin()) as f32;
            pass.push_frame(&vec![v; pass.channels]);
        }
    }

    #[test]
    fn k_weighted_997hz_full_scale_mono_reads_minus_3() {
        // BS.1770 calibration: a 0 dBFS 997 Hz sine in one channel reads −3.01 LKFS.
        let mut p = Pass::new(1);
        sine(&mut p, 997.0, 1.0, 5.0);
        let l = integrated(&window_energies(&p.subs, 4)).unwrap();
        assert!((l - -3.01).abs() < 0.05, "{l}");
    }

    #[test]
    fn stereo_minus_20_dbfs_sine_reads_minus_20() {
        let mut p = Pass::new(2);
        sine(&mut p, 1000.0, 0.1, 10.0);
        let l = integrated(&window_energies(&p.subs, 4)).unwrap();
        assert!((l - -20.0).abs() < 0.1, "{l}");
        assert!((db(p.sample_peak) - -20.0).abs() < 0.01);
        assert!((db(p.true_peak) - -20.0).abs() < 0.2, "{}", db(p.true_peak));
        // Steady tone: no loudness range.
        assert!(loudness_range(&window_energies(&p.subs, 30)) < 0.1);
    }

    #[test]
    fn relative_gate_ignores_quiet_passages_and_lra_measures_steps() {
        // 10 s at −20 dBFS then 10 s at −40 dBFS (more than 10 LU below): integrated stays ≈ −20.
        let mut p = Pass::new(2);
        sine(&mut p, 1000.0, 0.1, 10.0);
        sine(&mut p, 1000.0, 0.01, 10.0);
        let l = integrated(&window_energies(&p.subs, 4)).unwrap();
        assert!((l - -20.0).abs() < 0.2, "{l}");
        // −20 and −30 LUFS halves: LRA ≈ 10 LU.
        let mut q = Pass::new(2);
        sine(&mut q, 1000.0, 0.1, 20.0);
        sine(&mut q, 1000.0, 0.1 / 10f64.sqrt(), 20.0);
        let lra = loudness_range(&window_energies(&q.subs, 30));
        assert!((lra - 10.0).abs() < 0.6, "{lra}");
    }

    #[test]
    fn true_peak_catches_intersample_overs() {
        // fs/4 sine phased so every sample lands at ±0.707 of the true amplitude.
        let mut p = Pass::new(1);
        for i in 0..48_000 {
            let v = (std::f64::consts::PI / 2.0 * i as f64 + std::f64::consts::PI / 4.0).sin();
            p.push_frame(&[v as f32]);
        }
        assert!((db(p.sample_peak) - -3.01).abs() < 0.05);
        assert!(db(p.true_peak) > -0.5, "{}", db(p.true_peak));
    }
}
