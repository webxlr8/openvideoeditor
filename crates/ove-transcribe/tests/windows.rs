//! Audio is read for recognition in bounded, overlapping windows that cover the file exactly like a whole-file
//! decode. Uses the test-media matrix (no speech model needed).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

#[path = "../../../tests/media/fixtures.rs"]
mod fixtures;

use ove_transcribe::{AudioWindow, audio_windows, load_audio};

#[test]
fn windows_cover_the_audio_exactly_and_stay_bounded() {
    if !fixtures::media_ready() {
        return;
    }
    for name in ["audio_pcm.wav", "audio_aac.m4a", "h264_longgop.mp4"] {
        let path = fixtures::media(name);
        let whole = load_audio(&path).unwrap();
        // 3–4 s of audio in 1 s windows overlapping by 0.25 s.
        let (window, overlap) = (16_000, 4_000);
        let mut it = audio_windows(&path, window, overlap).unwrap();
        let windows: Vec<AudioWindow> = it.by_ref().map(Result::unwrap).collect();
        assert!(windows.len() >= 4, "{name}: {} windows", windows.len());
        assert!(windows.iter().all(|w| w.samples.len() <= window), "{name}");
        assert_eq!(windows.iter().filter(|w| w.last).count(), 1, "{name}");
        assert!(windows.last().unwrap().last);
        // Each window is exactly the matching slice of the whole-file decode.
        for w in &windows {
            assert_eq!(w.samples[..], whole[w.start..w.start + w.samples.len()], "{name}: window at {}", w.start);
        }
        let end = windows.last().map(|w| w.start + w.samples.len()).unwrap();
        assert_eq!(end, whole.len(), "{name}: windows reach the end");
        assert!(windows.windows(2).all(|p| p[1].start == p[0].start + window - overlap));
        // Memory: one window plus at most one decoded chunk.
        assert!(it.peak_buffered < window + 16_000, "{name}: buffered {}", it.peak_buffered);

        // Audio shorter than a window is a single window: the whole decode, as before.
        let one: Vec<AudioWindow> =
            audio_windows(&path, 16_000 * 30, 16_000 * 2).unwrap().map(Result::unwrap).collect();
        assert_eq!(one, vec![AudioWindow { start: 0, samples: whole.clone(), last: true }], "{name}");
    }
}

#[test]
fn files_without_sound_are_reported() {
    if !fixtures::media_ready() {
        return;
    }
    let e = audio_windows(&fixtures::media("h264_60fps.mov"), 16_000, 0).err().unwrap();
    assert!(matches!(e, ove_transcribe::TranscribeError::NoAudio), "{e}");
}
