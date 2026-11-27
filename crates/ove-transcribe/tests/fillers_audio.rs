//! add-transcript-editing 3.1 and 3.3 on real audio (ignored; needs local files):
//! - dead air on the speech fixture (speech, 2 s of silence, speech, 4 s of tones, speech) through the shared runtime:
//!   the silence is found and the tones are kept. Needs `OVE_VAD_DIR` (with `silero_vad.onnx`), `OVE_VAD_FIXTURE` and
//!   `OVE_ONNXRUNTIME_LIB`.
//! - fillers: a recording with 10 hesitations, transcribed with the verbatim option; at least 8 are found and no
//!   content word is selected. Needs `OVE_FILLER_FIXTURE` (16 kHz WAV) and `OVE_WHISPER_MODEL` (a ggml model file).
#![cfg(feature = "recognize")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ove_models::infer::Backend;
use ove_transcribe::cuts::Envelope;
use ove_transcribe::deadair::{DeadAirOptions, detect, fillers};

fn env(k: &str) -> PathBuf {
    PathBuf::from(std::env::var(k).unwrap_or_else(|_| panic!("set {k}")))
}

#[test]
#[ignore = "needs the voice-activity model, the fixture and the ONNX Runtime library"]
fn dead_air_on_real_speech_keeps_the_tones() {
    let samples = ove_transcribe::load_audio(&env("OVE_VAD_FIXTURE")).unwrap();
    let backend = ove_models::onnx::OnnxBackend::load(&env("OVE_ONNXRUNTIME_LIB")).unwrap().with_threads(1);
    let mut session = backend.load(&env("OVE_VAD_DIR").join("silero_vad.onnx")).unwrap();
    let mask = ove_transcribe::vad::onnx_mask(session.as_mut(), &samples).unwrap();
    let levels = Envelope::from_samples(&samples);
    let duration = samples.len() as i64 * 1_000_000 / 16_000;
    let d = detect(&mask, &levels, duration, DeadAirOptions::default());
    eprintln!("speech level {:?} dBFS; dead air {:?}", d.speech_db, d.ranges);
    assert!(d.speech_found);
    // The fixture's silence sits between the first two sentences; the 4 s of tones (between the second and third)
    // must not be cut. Sentence lengths come from the speech synthesiser, so find them from the mask.
    let speech = mask.ranges(ove_transcribe::vad::THRESHOLD);
    let tones_start = speech.iter().map(|r| r.1).filter(|e| *e > duration / 2).min().unwrap_or(0);
    assert!(!d.ranges.is_empty(), "the silence is found");
    for (a, b) in &d.ranges {
        let tone_overlap = (*b).min(tones_start + 4_000_000) - (*a).max(tones_start);
        assert!(tone_overlap < 500_000, "a cut {a}..{b} takes the tones");
    }
}

#[test]
#[ignore = "needs a recorded fixture with fillers and a speech model"]
fn verbatim_recognition_finds_most_fillers_and_no_content_word() {
    let opts = ove_transcribe::Options { verbatim: true, language: Some("en".into()), ..Default::default() };
    let t = ove_transcribe::transcribe_file(
        &env("OVE_WHISPER_MODEL"),
        &env("OVE_FILLER_FIXTURE"),
        &opts,
        |_| {},
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    eprintln!("{}", t.text());
    let f = fillers(&t, None);
    let selected: Vec<&str> = f.iter().filter(|x| x.selected).map(|x| x.text.as_str()).collect();
    eprintln!("selected: {selected:?}");
    assert!(selected.len() >= 8, "found {} of 10 fillers: {selected:?}", selected.len());
    assert!(
        selected.iter().all(|w| ove_transcribe::fillers::is_filler(w, "en")),
        "a content word was selected: {selected:?}"
    );
}
