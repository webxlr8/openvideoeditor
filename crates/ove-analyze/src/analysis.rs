//! The import-time analysis pass and its cache (design D2). One decode of a media file's audio produces:
//!
//! - the min/max peak pyramid ([`crate::peaks`]) for waveforms;
//! - the loudness summary (integrated loudness, loudness range, sample and true peak);
//! - an onset envelope: the rise in band-limited (about 150 Hz – 5 kHz) energy per 10 ms window, for later sync and
//!   beat analysis.
//!
//! Results live in the project bundle under `cache/waveforms/<key>.peaks` (pyramid and onset envelope) and
//! `<key>.json` (summary). The key is the media file's content id, so a file whose content changes gets a new key and
//! is analysed again. A file whose audio cannot be decoded gets `<key>.error` instead, so it is not retried on every
//! open. Later queries read these files and do not decode the file again.

use std::cell::Cell;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
use serde::{Deserialize, Serialize};

use crate::audio::{Loudness, Pass, RATE, loudness_of};
use crate::peaks::{PeakBuilder, PeakPyramid, PeakRange};
use crate::{Error, Result};

/// Version of the cached analysis; a summary of another version is analysed again.
pub const VERSION: u32 = 1;
/// Sample frames per onset-envelope value (10 ms).
pub const ONSET_HOP: u32 = RATE / 100;

thread_local! {
    static DECODES: Cell<usize> = const { Cell::new(0) };
}

/// Audio decodes started by analysis on this thread (for tests that check a file is decoded once).
pub fn decodes_on_this_thread() -> usize {
    DECODES.with(Cell::get)
}

pub(crate) fn note_decode() {
    DECODES.with(|d| d.set(d.get() + 1));
}

/// The cached summary of one media file's analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub version: u32,
    pub loudness: Loudness,
    /// Channels of the source audio.
    pub channels: usize,
    /// Sample frames at 48 kHz.
    pub frames: u64,
    /// Seconds.
    pub duration: f64,
    pub onset_hop: u32,
}

/// Everything one pass produces.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub summary: Summary,
    pub peaks: PeakPyramid,
    /// Onset strength per [`ONSET_HOP`] frames, 0–255 (scaled to the file's strongest onset).
    pub onset: Vec<u8>,
}

/// One-pole filters and the per-window energy behind the onset envelope.
struct Onset {
    hp_prev_x: f64,
    hp_y: f64,
    lp_y: f64,
    hp_a: f64,
    lp_a: f64,
    acc: f64,
    n: u32,
    last_log: f64,
    out: Vec<f64>,
}

impl Onset {
    fn new() -> Onset {
        let rc = |f: f64| 1.0 / (std::f64::consts::TAU * f);
        let dt = 1.0 / RATE as f64;
        Onset {
            hp_prev_x: 0.0,
            hp_y: 0.0,
            lp_y: 0.0,
            hp_a: rc(150.0) / (rc(150.0) + dt),
            lp_a: dt / (rc(5_000.0) + dt),
            acc: 0.0,
            n: 0,
            last_log: 0.0,
            out: Vec::new(),
        }
    }

    fn push(&mut self, mono: f64) {
        self.hp_y = self.hp_a * (self.hp_y + mono - self.hp_prev_x);
        self.hp_prev_x = mono;
        self.lp_y += self.lp_a * (self.hp_y - self.lp_y);
        self.acc += self.lp_y * self.lp_y;
        self.n += 1;
        if self.n == ONSET_HOP {
            self.close();
        }
    }

    fn close(&mut self) {
        if self.n == 0 {
            return;
        }
        let log = (self.acc / self.n as f64 + 1e-10).ln();
        self.out.push((log - self.last_log).max(0.0));
        self.last_log = log;
        self.acc = 0.0;
        self.n = 0;
    }

    fn finish(mut self) -> Vec<u8> {
        self.close();
        if let Some(first) = self.out.first_mut() {
            *first = 0.0; // The jump from silence before the file is not an onset.
        }
        let max = self.out.iter().copied().fold(0.0, f64::max);
        if max <= 0.0 {
            return vec![0; self.out.len()];
        }
        self.out.iter().map(|v| (v / max * 255.0).round() as u8).collect()
    }
}

/// Runs the analysis pass over a file's audio: one decode. `progress` gets the fraction done and returns false to
/// cancel, in which case the result is `Ok(None)`.
pub fn analyse_file(path: &Path, progress: &mut dyn FnMut(f64) -> bool) -> Result<Option<Analysis>> {
    ove_media::init();
    let info = ove_media::probe(path)?;
    let src = info.audio.as_ref().ok_or_else(|| Error::NoAudio(path.display().to_string()))?;
    let channels = src.channels.clamp(1, 8) as u16;
    let total = src.duration_us.or(info.duration_us).filter(|d| *d > 0).map(|d| d as f64 / 1e6 * RATE as f64);
    note_decode();
    let mut dec = AudioDecoder::open(path, AudioOutputSpec { sample_rate: RATE, channels, format: SampleFormat::F32 })?;
    let ch = channels as usize;
    let mut pass = Pass::new(ch);
    let mut peaks = PeakBuilder::new(ch);
    let mut onset = Onset::new();
    let mut frames = 0u64;
    let mut next_report = RATE as u64;
    while let Some(chunk) = dec.next_chunk()? {
        let Samples::F32(s) = &chunk.samples else { continue };
        for frame in s.chunks_exact(ch) {
            pass.push_frame(frame);
            peaks.push_frame(frame);
            onset.push(frame.iter().map(|v| *v as f64).sum::<f64>() / ch as f64);
        }
        frames += (s.len() / ch) as u64;
        if frames >= next_report {
            next_report = frames + RATE as u64;
            let f = total.map_or(0.0, |t| (frames as f64 / t).min(0.99));
            if !progress(f) {
                return Ok(None);
            }
        }
    }
    pass.close_window();
    let loudness = loudness_of(&pass);
    let summary =
        Summary { version: VERSION, channels: ch, frames, duration: loudness.duration, loudness, onset_hop: ONSET_HOP };
    Ok(Some(Analysis { summary, peaks: peaks.finish(), onset: onset.finish() }))
}

/// Where a media file's analysis lives in a bundle.
#[derive(Debug, Clone)]
pub struct CachePaths {
    pub peaks: PathBuf,
    pub summary: PathBuf,
    pub error: PathBuf,
}

/// The analysis cache folder of a bundle.
pub fn cache_dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("waveforms")
}

/// True for keys the cache accepts: 1–64 ASCII letters, digits, `-` or `_` (content ids are 32 hex digits).
pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn cache_paths(bundle: &Path, key: &str) -> CachePaths {
    let dir = cache_dir(bundle);
    CachePaths {
        peaks: dir.join(format!("{key}.peaks")),
        summary: dir.join(format!("{key}.json")),
        error: dir.join(format!("{key}.error")),
    }
}

/// Writes `bytes` through a part file and renames it into place, so readers never see half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = path.with_extension(format!("{}.part", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    let res = (|| {
        let mut f = std::fs::File::create(&part)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&part, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res
}

fn peaks_file(a: &Analysis) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    a.peaks.write_to(&mut bytes)?;
    bytes.write_all(&(a.onset.len() as u64).to_le_bytes())?;
    bytes.write_all(&a.onset)?;
    Ok(bytes)
}

/// Stores an analysis: the peak file first, then the summary, so a summary always has its peaks.
pub fn write_cache(bundle: &Path, key: &str, a: &Analysis) -> io::Result<()> {
    let p = cache_paths(bundle, key);
    write_atomic(&p.peaks, &peaks_file(a)?)?;
    let json = serde_json::to_vec_pretty(&a.summary).map_err(io::Error::other)?;
    write_atomic(&p.summary, &json)?;
    let _ = std::fs::remove_file(&p.error);
    Ok(())
}

/// Records that a file's audio could not be analysed.
pub fn write_error(bundle: &Path, key: &str, message: &str) -> io::Result<()> {
    write_atomic(&cache_paths(bundle, key).error, message.as_bytes())
}

/// State of a media file's analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Ready(Summary),
    Failed(String),
    Missing,
}

pub fn status(bundle: &Path, key: &str) -> Status {
    if !valid_key(key) {
        return Status::Missing;
    }
    let p = cache_paths(bundle, key);
    if let Some(s) = std::fs::read(&p.summary)
        .ok()
        .and_then(|b| serde_json::from_slice::<Summary>(&b).ok())
        .filter(|s| s.version == VERSION && p.peaks.is_file())
    {
        return Status::Ready(s);
    }
    match std::fs::read_to_string(&p.error) {
        Ok(m) => Status::Failed(m),
        Err(_) => Status::Missing,
    }
}

/// The cached peak pyramid and onset envelope.
pub fn read_peaks(bundle: &Path, key: &str) -> io::Result<(PeakPyramid, Vec<u8>)> {
    if !valid_key(key) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad analysis key"));
    }
    let mut f = io::BufReader::new(std::fs::File::open(cache_paths(bundle, key).peaks)?);
    let peaks = PeakPyramid::read_from(&mut f)?;
    let mut n = [0u8; 8];
    f.read_exact(&mut n)?;
    let n = u64::from_le_bytes(n) as usize;
    if n > 1 << 32 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad onset length"));
    }
    let mut onset = vec![0u8; n];
    f.read_exact(&mut onset)?;
    Ok((peaks, onset))
}

/// Peaks of a source range in seconds, in `bins` columns, from the cache.
pub fn peak_range(bundle: &Path, key: &str, start_s: f64, end_s: f64, bins: usize) -> io::Result<PeakRange> {
    let (p, _) = read_peaks(bundle, key)?;
    let r = crate::peaks::PEAK_RATE as f64;
    Ok(p.range(start_s.max(0.0) * r, end_s.max(start_s) * r, bins.clamp(1, 16_384)))
}

/// The analysis of a file, from the cache when present, else from one decode that is then cached. A file whose audio
/// cannot be decoded is recorded as failed and reported as [`Error::Media`] or [`Error::NoAudio`].
pub fn ensure(bundle: &Path, key: &str, path: &Path) -> Result<Summary> {
    if !valid_key(key) {
        return Err(Error::Invalid(format!("bad analysis key {key:?}")));
    }
    match status(bundle, key) {
        Status::Ready(s) => return Ok(s),
        Status::Failed(m) => return Err(Error::Invalid(m)),
        Status::Missing => {}
    }
    match analyse_file(path, &mut |_| true) {
        Ok(Some(a)) => {
            write_cache(bundle, key, &a).map_err(|e| Error::Invalid(format!("cannot store the analysis: {e}")))?;
            Ok(a.summary)
        }
        Ok(None) => Err(Error::Invalid("analysis cancelled".into())),
        Err(e) => {
            let _ = write_error(bundle, key, &e.to_string());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onset_envelope_marks_bursts() {
        let mut o = Onset::new();
        for i in 0..RATE as usize {
            // Silence, then a 1 kHz burst at 0.5 s.
            let v = if i >= RATE as usize / 2 {
                (i as f64 * 1000.0 * std::f64::consts::TAU / RATE as f64).sin()
            } else {
                0.0
            };
            o.push(v * 0.5);
        }
        let env = o.finish();
        assert_eq!(env.len(), 100);
        let peak = env.iter().enumerate().max_by_key(|e| e.1).unwrap().0;
        assert!((49..=51).contains(&peak), "strongest onset at window {peak}");
        assert_eq!(env[peak], 255);
        assert!(env[..48].iter().all(|v| *v == 0));
    }

    #[test]
    fn keys_are_checked() {
        assert!(valid_key("0123456789abcdef0123456789abcdef"));
        assert!(!valid_key("../x"));
        assert!(!valid_key(""));
        assert_eq!(status(Path::new("/nonexistent"), "../../etc"), Status::Missing);
    }
}
