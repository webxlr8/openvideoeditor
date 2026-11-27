//! Beats, downbeats, bars and tempo (add-audio-intelligence design D6, D7).
//!
//! The beat model runs on 22,050 Hz mono audio in 30 s chunks and gives two logits per 20 ms frame (50 frames per
//! second): beat and downbeat. This module holds everything around the model:
//!
//! - [`stitch`]: runs a frame model over overlapping chunks and keeps each chunk's middle, dropping border frames;
//! - [`pick`]: frames whose probability is at least 0.5 and a local maximum become beats and downbeats; each
//!   downbeat moves to its nearest beat;
//! - [`analyse`]: bars, the meter (beats per bar) and a tempo curve (median inter-beat interval per 8 s window);
//! - [`to_timeline`]: source seconds to timeline seconds through an item's trim, speed and reverse;
//! - the cache (`cache/beats/<key>.json`), which is never written into the project document.

use std::path::{Path, PathBuf};

use ove_core::model::Item;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Model id in the model store (`<models>/beat-this-small/`).
pub const MODEL_ID: &str = "beat-this-small";

/// One pinned model file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// The two graphs exported by tools/model-spike/export_beat_this.py from the Beat This `small0` checkpoint (MIT;
/// checkpoint SHA-256 6074be2c…7d47b, see tools/notices/beat-this.md).
pub const PINS: [Pin; 2] = [
    Pin {
        name: "beat_this_frontend.onnx",
        bytes: 4_467_329,
        sha256: "3372aaf12dc26b763c33dc0982d2ded866bcc569e84b3ffd38e75b8b08a1fb01",
    },
    Pin {
        name: "beat_this_small0.onnx",
        bytes: 10_555_597,
        sha256: "71793320633424fb60ad2361a9ff2ee1e7e0c654b1991446855a1cf99208c727",
    },
];

/// Sample rate the model takes.
pub const MODEL_RATE: u32 = 22_050;
/// Frames per second of the model's output.
pub const FPS: f64 = 50.0;
/// FFT size of the frontend; the piece is reflect-padded by half of it at both ends.
pub const N_FFT: usize = 1024;
/// Samples per frame.
pub const HOP: usize = 441;
/// Spectrogram frames per network chunk (30 s).
pub const CHUNK_FRAMES: usize = 1500;
/// Frames dropped at each chunk border (the network was not trained on them).
pub const BORDER_FRAMES: usize = 6;
/// Spectrogram frames per frontend run.
const FRONTEND_FRAMES: usize = 3000;
/// Peaks closer than this many frames are one peak.
const PEAK_RADIUS: usize = 3;
/// A downbeat moves to the nearest beat within this many seconds; further away it becomes a beat of its own.
const SNAP_S: f64 = 0.07;
/// Tempo-curve window, seconds.
const TEMPO_WINDOW_S: f64 = 8.0;
/// Version of the cached result.
pub const VERSION: u32 = 1;
/// Mel bands per frame.
pub const MELS: usize = 128;

/// The model's output for a stretch of audio: logits per frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Activations {
    pub beat: Vec<f32>,
    pub downbeat: Vec<f32>,
}

/// The frontend graph: reflect-padded samples to log-mel frames (`MELS` values each).
pub type Frontend<'a, E> = dyn FnMut(&[f32]) -> Result<Vec<f32>, E> + 'a;
/// The network graph: a spectrogram chunk of `frames` frames to logits.
pub type Network<'a, E> = dyn FnMut(&[f32], usize) -> Result<Activations, E> + 'a;

/// The whole pipeline around the two model graphs (tools/model-spike/export_beat_this.py):
///
/// 1. the piece is reflect-padded by `N_FFT / 2` and the `frontend` turns it into log-mel frames
///    (`MELS` values each), [`FRONTEND_FRAMES`] at a time;
/// 2. the `network` runs on [`CHUNK_FRAMES`]-frame chunks that start [`BORDER_FRAMES`] before the piece and are
///    `CHUNK_FRAMES − 2 · BORDER_FRAMES` apart, the last one moved to end at the piece's end, zero-padded at the
///    piece ends; each chunk's border frames are dropped and earlier chunks win where chunks overlap.
///
/// This is how the network was trained and evaluated; its logits depend on where chunks start.
pub fn run_pipeline<E>(
    samples: &[f32],
    frontend: &mut Frontend<'_, E>,
    network: &mut Network<'_, E>,
) -> Result<Activations, E> {
    if samples.len() < 2 {
        return Ok(Activations::default());
    }
    let half = N_FFT / 2;
    let n = samples.len();
    // Reflect padding (no edge repeat), as numpy and torch do it.
    let mut padded = Vec::with_capacity(n + 2 * half);
    padded.extend((1..=half).rev().map(|i| samples[i.min(n - 1)]));
    padded.extend_from_slice(samples);
    padded.extend((0..half).map(|i| samples[n.saturating_sub(2 + i)]));
    let frames = 1 + n / HOP;
    let mut spect = Vec::with_capacity(frames * MELS);
    let mut f0 = 0;
    while f0 < frames {
        let f1 = frames.min(f0 + FRONTEND_FRAMES);
        let seg = &padded[f0 * HOP..((f1 - 1) * HOP + N_FFT).min(padded.len())];
        let part = frontend(seg)?;
        spect.extend_from_slice(&part[..((f1 - f0) * MELS).min(part.len())]);
        f0 = f1;
    }
    let frames = spect.len() / MELS;
    let step = (CHUNK_FRAMES - 2 * BORDER_FRAMES) as i64;
    let b = BORDER_FRAMES as i64;
    let total = frames as i64;
    let mut starts: Vec<i64> = (0..).map(|k| -b + k * step).take_while(|s| *s < total - b).collect();
    if total > step
        && let Some(last) = starts.last_mut()
    {
        *last = total - (CHUNK_FRAMES as i64 - b);
    }
    let mut out = Activations { beat: vec![-1000.0; frames], downbeat: vec![-1000.0; frames] };
    for &start in starts.iter().rev() {
        let (lo, hi) = (start.max(0), (start + CHUNK_FRAMES as i64).min(total));
        let left = (-start).max(0) as usize;
        let right = (start + CHUNK_FRAMES as i64 - total).clamp(0, b) as usize;
        let len = left + (hi - lo) as usize + right;
        let mut chunk = vec![0.0f32; len * MELS];
        chunk[left * MELS..(left + (hi - lo) as usize) * MELS]
            .copy_from_slice(&spect[lo as usize * MELS..hi as usize * MELS]);
        let act = network(&chunk, len)?;
        let keep = act.beat.len().min(act.downbeat.len()).saturating_sub(2 * BORDER_FRAMES);
        for k in 0..keep {
            let f = start + b + k as i64;
            if (0..total).contains(&f) {
                out.beat[f as usize] = act.beat[BORDER_FRAMES + k];
                out.downbeat[f as usize] = act.downbeat[BORDER_FRAMES + k];
            }
        }
    }
    Ok(out)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Frames whose probability is at least 0.5 and the highest within [`PEAK_RADIUS`] (the earliest of equal ones).
fn peaks(logits: &[f32]) -> Vec<usize> {
    let mut out = Vec::new();
    for i in 0..logits.len() {
        if sigmoid(logits[i]) < 0.5 {
            continue;
        }
        let lo = i.saturating_sub(PEAK_RADIUS);
        let hi = (i + PEAK_RADIUS + 1).min(logits.len());
        let best = (lo..hi).all(|j| logits[j] < logits[i] || (logits[j] == logits[i] && j >= i));
        if best {
            out.push(i);
        }
    }
    out
}

/// Beats and downbeats in seconds from frame logits. Every downbeat is also a beat.
pub fn pick(a: &Activations) -> (Vec<f64>, Vec<f64>) {
    let t = |f: usize| f as f64 / FPS;
    let mut beats: Vec<f64> = peaks(&a.beat).into_iter().map(t).collect();
    let mut downs = Vec::new();
    for d in peaks(&a.downbeat).into_iter().map(t) {
        let near = beats.iter().copied().min_by(|x, y| (x - d).abs().total_cmp(&(y - d).abs()));
        match near {
            Some(b) if (b - d).abs() <= SNAP_S => downs.push(b),
            _ => {
                beats.push(d);
                downs.push(d);
            }
        }
    }
    beats.sort_by(f64::total_cmp);
    beats.dedup();
    downs.sort_by(f64::total_cmp);
    downs.dedup();
    (beats, downs)
}

/// One bar: from a downbeat to the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bar {
    pub start: f64,
    pub end: f64,
    pub beats: u32,
}

/// Tempo over one window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoPoint {
    pub at: f64,
    pub bpm: f64,
}

/// Everything detected in a media file, in source seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatAnalysis {
    pub version: u32,
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
    pub bars: Vec<Bar>,
    /// Beats per bar (the most common count), when there are at least two bars.
    pub meter: Option<u32>,
    /// Median tempo, beats per minute.
    pub bpm: Option<f64>,
    pub tempo: Vec<TempoPoint>,
    /// Model id that produced it.
    pub model: String,
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) })
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Bars, meter and tempo from beats and downbeats (seconds).
pub fn analyse(beats: &[f64], downbeats: &[f64], model: &str) -> BeatAnalysis {
    let bars: Vec<Bar> = downbeats
        .windows(2)
        .map(|w| Bar {
            start: r3(w[0]),
            end: r3(w[1]),
            beats: beats.iter().filter(|b| **b >= w[0] - 1e-9 && **b < w[1] - 1e-9).count() as u32,
        })
        .collect();
    let meter = if bars.len() >= 2 {
        let mut counts = std::collections::BTreeMap::<u32, usize>::new();
        for b in &bars {
            *counts.entry(b.beats).or_default() += 1;
        }
        counts.into_iter().max_by_key(|(k, n)| (*n, std::cmp::Reverse(*k))).map(|(k, _)| k)
    } else {
        None
    };
    let ibi = |lo: f64, hi: f64| -> Option<f64> {
        let mut d: Vec<f64> =
            beats.windows(2).filter(|w| w[0] >= lo && w[1] <= hi).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
        median(&mut d)
    };
    let end = beats.last().copied().unwrap_or(0.0);
    let mut tempo = Vec::new();
    let mut at = 0.0;
    while at < end {
        if let Some(i) = ibi(at, at + TEMPO_WINDOW_S) {
            tempo.push(TempoPoint { at: r3(at), bpm: (60.0 / i * 100.0).round() / 100.0 });
        }
        at += TEMPO_WINDOW_S;
    }
    BeatAnalysis {
        version: VERSION,
        beats: beats.iter().map(|b| r3(*b)).collect(),
        downbeats: downbeats.iter().map(|b| r3(*b)).collect(),
        bars,
        meter,
        bpm: ibi(f64::NEG_INFINITY, f64::INFINITY).map(|i| (60.0 / i * 100.0).round() / 100.0),
        tempo,
        model: model.to_string(),
    }
}

/// Source seconds to timeline seconds for an item, through trim, speed and reverse. Times outside the item's
/// visible part are dropped; the result is ascending.
pub fn to_timeline(item: &Item, source: &[f64]) -> Vec<f64> {
    let speed = item.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01);
    let reverse = item.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse");
    let trim = item.time_field("trimStart") as f64 / 1e6;
    let (start, end) = (item.start as f64 / 1e6, item.end as f64 / 1e6);
    let used = (end - start) * speed;
    let mut out: Vec<f64> = source
        .iter()
        .filter(|s| **s >= trim - 1e-9 && **s <= trim + used + 1e-9)
        .map(|s| if reverse { start + (trim + used - s) / speed } else { start + (s - trim) / speed })
        .filter(|t| *t >= start - 1e-9 && *t < end - 1e-9)
        .map(|t| (t * 1e6).round() / 1e6)
        .collect();
    out.sort_by(f64::total_cmp);
    out
}

/// Where a media file's beats are cached.
pub fn cache_path(bundle: &Path, key: &str) -> PathBuf {
    bundle.join("cache").join("beats").join(format!("{key}.json"))
}

/// The cached analysis, when present and of this version.
pub fn load(bundle: &Path, key: &str) -> Option<BeatAnalysis> {
    if !crate::analysis::valid_key(key) {
        return None;
    }
    let a: BeatAnalysis = serde_json::from_slice(&std::fs::read(cache_path(bundle, key)).ok()?).ok()?;
    (a.version == VERSION).then_some(a)
}

/// Stores an analysis (through a part file, so readers never see half a file).
pub fn store(bundle: &Path, key: &str, a: &BeatAnalysis) -> std::io::Result<()> {
    if !crate::analysis::valid_key(key) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad cache key"));
    }
    let path = cache_path(bundle, key);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let part = path.with_extension("json.part");
    std::fs::write(&part, serde_json::to_vec_pretty(a).map_err(std::io::Error::other)?)?;
    std::fs::rename(&part, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Logits with a peak at every `period` frames from `phase`, and downbeat peaks every `bar` beats.
    fn clicks(frames: usize, period: usize, bar: usize) -> Activations {
        let mut a = Activations { beat: vec![-6.0; frames], downbeat: vec![-6.0; frames] };
        for (k, f) in (0..frames).step_by(period).enumerate() {
            a.beat[f] = 4.0;
            if f + 1 < frames {
                a.beat[f + 1] = 1.0; // a shoulder, not a second peak
            }
            if k % bar == 0 {
                // Downbeat peak a frame late: it moves onto the beat.
                a.downbeat[(f + 1).min(frames - 1)] = 3.0;
            }
        }
        a
    }

    #[test]
    fn a_steady_120_bpm_grid_gives_beats_every_500_ms_in_four_four() {
        let (b, d) = pick(&clicks(50 * 60, 25, 4));
        let a = analyse(&b, &d, "test");
        assert_eq!(a.beats.len(), 120);
        assert!(a.beats.windows(2).all(|w| (w[1] - w[0] - 0.5).abs() < 1e-9));
        assert_eq!(a.downbeats.len(), 30);
        assert_eq!(a.meter, Some(4));
        assert_eq!(a.bpm, Some(120.0));
        assert!(a.tempo.iter().all(|t| (t.bpm - 120.0).abs() < 0.01));
        assert!(a.downbeats.iter().all(|d| a.beats.contains(d)));
    }

    #[test]
    fn three_four_has_a_downbeat_every_third_beat() {
        let (b, d) = pick(&clicks(50 * 30, 30, 3));
        let a = analyse(&b, &d, "test");
        assert_eq!(a.meter, Some(3));
        assert!(a.bars.iter().all(|b| b.beats == 3), "{:?}", a.bars);
        assert_eq!(a.bpm, Some(100.0));
    }

    #[test]
    fn chunks_start_before_the_piece_and_earlier_chunks_win() {
        // Frontend: one mel row per frame holding the frame's centre sample. Network: logit = row value, except a
        // chunk marks its own index in the downbeat output so overlaps can be told apart.
        let samples: Vec<f32> = (0..(70 * MODEL_RATE as usize)).map(|i| (i / HOP) as f32).collect();
        let mut front = |s: &[f32]| -> Result<Vec<f32>, ()> {
            let n = (s.len() - N_FFT) / HOP + 1;
            Ok((0..n).flat_map(|k| std::iter::repeat_n(s[k * HOP + N_FFT / 2], MELS)).collect())
        };
        let mut calls = 0;
        let mut net = |c: &[f32], len: usize| -> Result<Activations, ()> {
            calls += 1;
            assert!(len <= CHUNK_FRAMES);
            let beat: Vec<f32> = (0..len).map(|k| c[k * MELS]).collect();
            Ok(Activations { downbeat: vec![calls as f32; len], beat })
        };
        let a = run_pipeline(&samples, &mut front, &mut net).unwrap();
        let frames = 1 + samples.len() / HOP;
        assert_eq!(a.beat.len(), frames);
        // Every frame comes from a chunk where it was not a border frame, so it holds its own centre sample.
        for (k, v) in a.beat.iter().enumerate().take(frames - 1) {
            assert_eq!(*v, k as f32, "frame {k}");
        }
        // 3,501 frames: chunks at −6, 1482 and the last moved to 2007; chunks run last to first, so the first
        // chunk (called third) wins where it overlaps.
        assert_eq!(calls, 3);
        assert_eq!(a.downbeat[0], 3.0);
        assert_eq!(a.downbeat[frames - 1], 1.0);
    }
}
