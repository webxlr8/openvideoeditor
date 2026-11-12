//! End-to-end recognition on synthetic speech (espeak-ng + system ffmpeg generate the clip). Needs the base model:
//! set `OVE_WHISPER_MODEL_DIR` to a folder holding `ggml-base.bin` (downloaded if missing) — run with `--ignored`.
//! Without espeak-ng or ffmpeg the test fails, unless `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` makes it print "skipped".

#[path = "../../../tests/media/fixtures.rs"]
mod fixtures;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ove_transcribe::{Model, Options, SegmentOptions, load_audio, models, segment, transcribe};

const SCRIPT: &str = "The quick brown fox jumps over the lazy dog. Captions make every video easier to follow.";

fn speech_clip(dir: &Path) -> Option<PathBuf> {
    let ffmpeg = fixtures::fixture_ffmpeg();
    if !fixtures::require_tool("espeak-ng", "it synthesises the test speech")
        || !fixtures::require_tool(&ffmpeg, "it muxes the test clip; set OVE_FIXTURE_FFMPEG")
    {
        return None;
    }
    let wav = dir.join("speech.wav");
    let ok = Command::new("espeak-ng").args(["-s", "150", "-w"]).arg(&wav).arg(SCRIPT).status().unwrap().success();
    assert!(ok, "espeak-ng failed");
    // Put the speech 1 s into a video file, as it would be in real footage.
    let mp4 = dir.join("speech.mp4");
    let st = Command::new(ffmpeg)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=gray:s=320x180:r=25:d=9", "-i"])
        .arg(&wav)
        .args([
            "-filter_complex",
            "[1:a]adelay=1000|1000,apad[a]",
            "-map",
            "0:v",
            "-map",
            "[a]",
            "-t",
            "9",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ])
        .arg(&mp4)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg failed to make the speech clip");
    Some(mp4)
}

fn norm(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word error rate by edit distance.
fn wer(reference: &[String], hyp: &[String]) -> f64 {
    let (n, m) = (reference.len(), hyp.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let c = if reference[i - 1] == hyp[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + c);
        }
    }
    d[n][m] as f64 / n.max(1) as f64
}

#[test]
#[ignore = "needs the speech model (~148 MB download) and espeak-ng"]
fn recognises_synthetic_speech_with_word_timings() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(clip) = speech_clip(tmp.path()) else { return };
    let dir = models::model_dir(tmp.path());
    let path = models::ensure_model(&dir, Model::Base, &mut |_, _| {}, &AtomicBool::new(false)).unwrap();
    let audio = load_audio(&clip).unwrap();
    assert!((audio.len() as f64 / 16_000.0 - 9.0).abs() < 0.2, "{} samples", audio.len());
    let t0 = std::time::Instant::now();
    let tr = transcribe(
        &path,
        &audio,
        &Options { language: Some("en".into()), ..Default::default() },
        |_| {},
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let took = t0.elapsed();
    eprintln!("TRANSCRIBE 9 s of audio in {took:?} ({:.1}× real time): {}", 9.0 / took.as_secs_f64(), tr.text());
    let e = wer(&norm(SCRIPT), &norm(&tr.text()));
    // Synthetic (espeak) speech is harder than a human voice; the clear phrases must come through.
    assert!(e < 0.5, "word error rate {e:.2}: {}", tr.text());
    let lower = tr.text().to_lowercase();
    for phrase in ["quick brown fox", "lazy dog", "captions", "easier to follow"] {
        assert!(lower.contains(phrase), "missing {phrase:?}: {lower}");
    }
    // Speech starts after the 1 s of silence; timestamps are monotonic and inside the clip.
    assert!(tr.words[0].start_us >= 700_000, "first word at {}", tr.words[0].start_us);
    for w in tr.words.windows(2) {
        assert!(w[0].start_us <= w[1].start_us && w[0].end_us <= w[1].start_us + 1, "{w:?}");
    }
    assert!(tr.words.last().unwrap().end_us <= 9_100_000);
    let segs = segment(&tr.words, &SegmentOptions::default());
    assert!(segs.len() >= 2, "{segs:?}");
    assert!(segs[0].text.to_lowercase().contains("fox"));
}
