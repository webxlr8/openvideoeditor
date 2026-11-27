//! add-transcript-editing task 1.1: the two voice-activity paths on the same fixture: the recognition library's
//! built-in voice activity (ggml weights) and the ONNX export through the shared inference runtime. Every speech run
//! of the runtime path must match a run of the built-in path within one 32 ms cell at both ends; the timings and any
//! runs only one path finds are printed for design.md. Needs:
//! - `OVE_VAD_DIR`: a folder with `ggml-silero-v6.2.0.bin` and `silero_vad.onnx` (the pinned files);
//! - `OVE_VAD_FIXTURE`: a 16 kHz mono WAV with speech and pauses;
//! - `OVE_ONNXRUNTIME_LIB`: the ONNX Runtime library.
#![cfg(feature = "recognize")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use std::path::PathBuf;
use std::time::Instant;

use ove_models::infer::Backend;
use ove_transcribe::vad::{THRESHOLD, onnx_mask, whisper_mask};

#[test]
#[ignore = "needs the voice-activity models, a fixture and the ONNX Runtime library (see the file header)"]
fn both_voice_activity_paths_agree_within_one_cell() {
    let dir = PathBuf::from(std::env::var("OVE_VAD_DIR").expect("set OVE_VAD_DIR"));
    let fixture = PathBuf::from(std::env::var("OVE_VAD_FIXTURE").expect("set OVE_VAD_FIXTURE"));
    let lib = PathBuf::from(std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB"));
    let samples = ove_transcribe::load_audio(&fixture).unwrap();
    let seconds = samples.len() as f64 / 16_000.0;

    let t = Instant::now();
    let a = whisper_mask(&dir.join("ggml-silero-v6.2.0.bin"), &samples).unwrap();
    let whisper_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let backend = ove_models::onnx::OnnxBackend::load(&lib).unwrap().with_threads(1);
    let mut session = backend.load(&dir.join("silero_vad.onnx")).unwrap();
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let b = onnx_mask(session.as_mut(), &samples).unwrap();
    let onnx_ms = t.elapsed().as_secs_f64() * 1000.0;

    let diff = a.compare(&b, THRESHOLD);
    eprintln!(
        "{seconds:.1} s of audio, {} cells: built-in {whisper_ms:.1} ms; runtime {onnx_ms:.1} ms (+{load_ms:.1} ms to load \
         the runtime and model); {} cells differ, boundaries moved at most {} cell(s)",
        diff.cells, diff.differing, diff.max_shift
    );
    eprintln!("built-in speech: {:?}", a.ranges(THRESHOLD));
    eprintln!("runtime speech:  {:?}", b.ranges(THRESHOLD));
    assert!(a.probs.len().abs_diff(b.probs.len()) <= 1, "{} vs {} cells", a.probs.len(), b.probs.len());
    // Every speech run the runtime path finds is found by the built-in path with both ends within one cell.
    let (ra, rb) = (a.ranges(THRESHOLD), b.ranges(THRESHOLD));
    assert!(!rb.is_empty(), "speech was found");
    let cell = ove_transcribe::vad::CELL_US;
    for (s, e) in &rb {
        assert!(
            ra.iter().any(|(s2, e2)| (s - s2).abs() <= cell && (e - e2).abs() <= cell),
            "runtime run {s}..{e} has no built-in run within one cell: {ra:?}"
        );
    }
    // Runs only the built-in path finds (recorded in design.md R1 of add-transcript-editing).
    let extra: Vec<_> = ra
        .iter()
        .filter(|(s, e)| !rb.iter().any(|(s2, e2)| (s - s2).abs() <= cell && (e - e2).abs() <= cell))
        .collect();
    eprintln!("built-in only: {extra:?}");
}
