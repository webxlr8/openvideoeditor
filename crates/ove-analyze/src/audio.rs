//! Audio analysis in one streaming pass over the decoded file (48 kHz float, the source's channel count): peak and
//! RMS levels, waveform images, silence ranges, and loudness per ITU-R BS.1770-4 / EBU R128 (integrated loudness
//! with absolute and relative gating, loudness range per EBU Tech 3342, sample peak and a 4× oversampled true-peak
//! estimate).
//!
//! Levels in dB are floored at [`FLOOR_DB`] (digital silence).

use std::path::Path;

use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
use ove_render::loudness::{
    Biquad, TruePeak, channel_weights, integrated, k_weighting, loudness_range, lufs, window_energies,
};
use serde::{Deserialize, Serialize};

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

/// Everything the public functions need, from one decode.
pub(crate) struct Pass {
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
    peaks: Vec<TruePeak>,
    frames: u64,
}

impl Pass {
    pub(crate) fn new(channels: usize) -> Pass {
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
            peaks: vec![TruePeak::default(); channels],
            frames: 0,
        }
    }

    pub(crate) fn push_frame(&mut self, frame: &[f32]) {
        let sub_len = (RATE / 10) as usize;
        for (c, &s) in frame.iter().enumerate().take(self.channels) {
            let x = s as f64;
            let a = x.abs();
            self.win_peak = self.win_peak.max(a);
            self.win_sq += x * x;
            self.sample_peak = self.sample_peak.max(a);
            self.true_peak = self.true_peak.max(self.peaks[c].push(x));
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

    pub(crate) fn close_window(&mut self) {
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
    crate::analysis::note_decode();
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Loudness of a file's audio.
pub fn loudness(path: &Path) -> Result<Loudness> {
    Ok(loudness_of(&analyse(path)?))
}

/// The loudness summary of a finished pass.
pub(crate) fn loudness_of(pass: &Pass) -> Loudness {
    let blocks = window_energies(&pass.subs, 4);
    let short = window_energies(&pass.subs, 30);
    let max_short = short
        .iter()
        .copied()
        .filter(|e| *e > 0.0)
        .map(lufs)
        .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    Loudness {
        integrated_lufs: integrated(&blocks).map(r2),
        loudness_range_lu: r2(loudness_range(&short)),
        max_short_term_lufs: max_short.map(r2),
        sample_peak_dbfs: r2(db(pass.sample_peak)),
        true_peak_dbtp: r2(db(pass.true_peak.max(pass.sample_peak))),
        channels: pass.channels,
        duration: (pass.duration() * 1000.0).round() / 1000.0,
    }
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
