//! Voice activity: a speech probability for every 32 ms cell of 16 kHz mono audio (add-transcript-editing D5, D6).
//!
//! Two ways to run the Silero voice-activity model were compared (task 1.1, design.md Results): the voice-activity
//! support built into the recognition library (ggml weights, no extra runtime) and the ONNX export through the shared
//! inference runtime (`ove-models`). Both stay here so the comparison stays testable; features use [`onnx_mask`],
//! the chosen path (37 times faster, and no false speech in a music bed).

#[cfg(feature = "recognize")]
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Result, TranscribeError};
use crate::transcript::SAMPLE_RATE;

/// Samples in one cell (32 ms at 16 kHz).
pub const CELL_SAMPLES: usize = 512;
/// One cell in microseconds.
pub const CELL_US: i64 = CELL_SAMPLES as i64 * 1_000_000 / SAMPLE_RATE as i64;
/// Probability at or above which a cell counts as speech.
pub const THRESHOLD: f32 = 0.5;

/// Speech probabilities per cell, from the start of the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechMask {
    pub cell_us: i64,
    pub probs: Vec<f32>,
}

impl SpeechMask {
    /// True for each cell at or above `threshold`.
    pub fn cells(&self, threshold: f32) -> Vec<bool> {
        self.probs.iter().map(|p| *p >= threshold).collect()
    }

    /// Speech runs as `(start_us, end_us)`.
    pub fn ranges(&self, threshold: f32) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        for (i, s) in self.cells(threshold).into_iter().chain(std::iter::once(false)).enumerate() {
            match (s, start) {
                (true, None) => start = Some(i),
                (false, Some(a)) => {
                    out.push((a as i64 * self.cell_us, i as i64 * self.cell_us));
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    /// Cells where two masks disagree, and how far the boundaries moved: [`MaskDiff::max_shift`] is 1 when every
    /// disagreement is one cell next to a speech boundary of `self`. Masks of different lengths are compared over the
    /// shorter one.
    pub fn compare(&self, other: &SpeechMask, threshold: f32) -> MaskDiff {
        let (a, b) = (self.cells(threshold), other.cells(threshold));
        let n = a.len().min(b.len());
        let edges: Vec<usize> = (1..n).filter(|&i| a[i] != a[i - 1]).collect();
        let mut diff = MaskDiff { cells: n, differing: 0, max_shift: 0 };
        for i in 0..n {
            if a[i] != b[i] {
                diff.differing += 1;
                // The cells on both sides of a boundary count as at the boundary.
                let d = edges.iter().map(|&e| e.abs_diff(i).min((e - 1).abs_diff(i))).min();
                diff.max_shift = diff.max_shift.max(d.map_or(usize::MAX, |d| d + 1));
            }
        }
        diff
    }
}

/// The result of [`SpeechMask::compare`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskDiff {
    pub cells: usize,
    pub differing: usize,
    /// How many cells a speech boundary moved at most (0 when the masks agree).
    pub max_shift: usize,
}

/// The speech mask from the recognition library's built-in voice activity (Silero, ggml weights at `model`).
#[cfg(feature = "recognize")]
pub fn whisper_mask(model: &Path, samples: &[f32]) -> Result<SpeechMask> {
    use whisper_rs::{WhisperVadContext, WhisperVadContextParams};
    let path = model.to_str().filter(|p| !p.contains('\0')).ok_or_else(|| {
        TranscribeError::Model(format!("the voice-activity model path {} is not usable", model.display()))
    })?;
    let mut params = WhisperVadContextParams::new();
    params.set_n_threads(std::thread::available_parallelism().map(|n| n.get().min(4) as i32).unwrap_or(1));
    params.set_use_gpu(false);
    let mut ctx = WhisperVadContext::new(path, params)
        .map_err(|e| TranscribeError::Model(format!("could not load the voice-activity model: {e}")))?;
    if samples.is_empty() {
        return Ok(SpeechMask { cell_us: CELL_US, probs: Vec::new() });
    }
    ctx.detect_speech(samples).map_err(|e| TranscribeError::Model(format!("voice activity failed: {e}")))?;
    Ok(SpeechMask { cell_us: CELL_US, probs: ctx.probabilities().to_vec() })
}

/// Samples of context the ONNX export takes before each cell.
pub const ONNX_CONTEXT: usize = 64;

/// The speech mask from the Silero ONNX export run through the shared inference runtime: one call per cell with
/// 64 samples of context, the recurrent state carried between calls.
pub fn onnx_mask(session: &mut dyn ove_models::infer::Session, samples: &[f32]) -> Result<SpeechMask> {
    let mut v = OnnxVad::default();
    v.push(session, samples)?;
    v.finish(session)
}

/// [`onnx_mask`] over audio that arrives in chunks of any length (long files are never held in memory).
#[derive(Debug, Default)]
pub struct OnnxVad {
    state: Vec<f32>,
    context: Vec<f32>,
    pending: Vec<f32>,
    probs: Vec<f32>,
}

impl OnnxVad {
    pub fn push(&mut self, session: &mut dyn ove_models::infer::Session, samples: &[f32]) -> Result<()> {
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / CELL_SAMPLES * CELL_SAMPLES;
        let cells: Vec<f32> = self.pending.drain(..whole).collect();
        for chunk in cells.chunks(CELL_SAMPLES) {
            self.step(session, chunk)?;
        }
        Ok(())
    }

    pub fn finish(mut self, session: &mut dyn ove_models::infer::Session) -> Result<SpeechMask> {
        if !self.pending.is_empty() {
            let last = std::mem::take(&mut self.pending);
            self.step(session, &last)?;
        }
        Ok(SpeechMask { cell_us: CELL_US, probs: self.probs })
    }

    fn step(&mut self, session: &mut dyn ove_models::infer::Session, chunk: &[f32]) -> Result<()> {
        use ove_models::infer::Tensor;
        if self.state.is_empty() {
            self.state = vec![0f32; 2 * 128];
            self.context = vec![0f32; ONNX_CONTEXT];
        }
        let mut input = Vec::with_capacity(ONNX_CONTEXT + CELL_SAMPLES);
        input.extend_from_slice(&self.context);
        input.extend_from_slice(chunk);
        input.resize(ONNX_CONTEXT + CELL_SAMPLES, 0.0);
        self.context.copy_from_slice(&input[input.len() - ONNX_CONTEXT..]);
        let out = session
            .run(&[
                Tensor::f32("input", &[1, ONNX_CONTEXT + CELL_SAMPLES], input),
                Tensor::f32("state", &[2, 1, 128], self.state.clone()),
                Tensor::i64("sr", &[], vec![i64::from(SAMPLE_RATE)]),
            ])
            .map_err(|e| TranscribeError::Model(format!("voice activity failed: {e}")))?;
        let p = out.iter().find(|t| t.name == "output").and_then(|t| t.as_f32()).and_then(|v| v.first().copied());
        let s = out.iter().find(|t| t.name == "stateN").and_then(|t| t.as_f32());
        match (p, s) {
            (Some(p), Some(s)) if s.len() == self.state.len() => {
                self.probs.push(p);
                self.state.copy_from_slice(s);
                Ok(())
            }
            _ => Err(TranscribeError::Model("the voice-activity model gave unexpected outputs".into())),
        }
    }
}

fn cache_path(bundle: &std::path::Path, key: &str) -> std::path::PathBuf {
    bundle.join("cache").join("vad").join(format!("{key}.json"))
}

/// The cached speech mask of a media file (by content identity).
pub fn load(bundle: &std::path::Path, key: &str) -> Option<SpeechMask> {
    serde_json::from_slice(&std::fs::read(cache_path(bundle, key)).ok()?).ok()
}

pub fn store(bundle: &std::path::Path, key: &str, mask: &SpeechMask) {
    let p = cache_path(bundle, key);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(mask).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(bits: &str) -> SpeechMask {
        SpeechMask { cell_us: CELL_US, probs: bits.chars().map(|c| if c == '1' { 0.9 } else { 0.1 }).collect() }
    }

    #[test]
    fn ranges_cover_speech_runs() {
        assert_eq!(CELL_US, 32_000);
        assert_eq!(mask("0110001").ranges(THRESHOLD), vec![(32_000, 96_000), (192_000, 224_000)]);
        assert!(mask("000").ranges(THRESHOLD).is_empty());
    }

    #[test]
    fn disagreements_at_an_edge_are_within_one_cell() {
        let d = mask("0011110000").compare(&mask("0001111000"), THRESHOLD);
        assert_eq!((d.differing, d.max_shift), (2, 1));
        let d = mask("0011110000").compare(&mask("0111100000"), THRESHOLD);
        assert_eq!((d.differing, d.max_shift), (2, 1));
        let d = mask("0011110000").compare(&mask("0000110000"), THRESHOLD);
        assert_eq!(d.max_shift, 2);
        let d = mask("0011110000").compare(&mask("0011110011"), THRESHOLD);
        assert!(d.max_shift > 1);
        assert_eq!(mask("0110").compare(&mask("0110"), THRESHOLD).max_shift, 0);
    }
}
