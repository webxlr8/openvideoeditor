//! End-to-end recognition on synthetic speech (espeak-ng + system ffmpeg generate the clip). Needs the base model:
//! set `OVE_WHISPER_MODEL_DIR` to a folder holding `ggml-base.bin` (downloaded if missing) — run with `--ignored`.
//! Without espeak-ng or ffmpeg the test fails, unless `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` makes it print "skipped".
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

#[path = "../../../tests/media/fixtures.rs"]
mod fixtures;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ove_transcribe::{Model, Options, SegmentOptions, load_audio, models, segment, transcribe, transcribe_file};

const SCRIPT: &str = "The quick brown fox jumps over the lazy dog. Captions make every video easier to follow.";

/// The script spoken once 1 s into a 9 s video.
fn speech_clip(dir: &Path) -> Option<PathBuf> {
    speech_clip_at(dir, &[1000], 9)
}

/// The script spoken at each of `starts_ms` in a `secs`-long video.
fn speech_clip_at(dir: &Path, starts_ms: &[u32], secs: u32) -> Option<PathBuf> {
    let ffmpeg = fixtures::fixture_ffmpeg();
    if !fixtures::require_tool("espeak-ng", "it synthesises the test speech")
        || !fixtures::require_tool(&ffmpeg, "it muxes the test clip; set OVE_FIXTURE_FFMPEG")
    {
        return None;
    }
    let wav = dir.join("speech.wav");
    let ok = Command::new("espeak-ng").args(["-s", "150", "-w"]).arg(&wav).arg(SCRIPT).status().unwrap().success();
    assert!(ok, "espeak-ng failed");
    // Put the speech into a video file, as it would be in real footage.
    let mp4 = dir.join(format!("speech-{secs}.mp4"));
    let n = starts_ms.len();
    let mut graph = format!("[1:a]asplit={n}");
    graph += &(0..n).map(|i| format!("[s{i}]")).collect::<String>();
    for (i, ms) in starts_ms.iter().enumerate() {
        graph += &format!(";[s{i}]adelay={ms}|{ms}[d{i}]");
    }
    graph += ";";
    graph += &(0..n).map(|i| format!("[d{i}]")).collect::<String>();
    graph += &format!("amix=inputs={n}:normalize=0,apad[a]");
    let st = Command::new(ffmpeg)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=gray:s=320x180:r=25:d={secs}"))
        .arg("-i")
        .arg(&wav)
        .args(["-filter_complex", &graph, "-map", "0:v", "-map", "[a]", "-t"])
        .arg(secs.to_string())
        .args(["-c:v", "mpeg4", "-c:a", "aac"])
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
    // Streaming the file window by window gives the same transcript as the whole-file decode.
    let streamed = transcribe_file(
        &path,
        &clip,
        &Options { language: Some("en".into()), ..Default::default() },
        |_| {},
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(streamed, tr);
}

#[test]
#[ignore = "needs the speech model (~148 MB download) and espeak-ng"]
fn long_audio_is_recognised_across_windows() {
    let tmp = tempfile::tempdir().unwrap();
    // The second utterance (~6 s) runs across the 30 s window boundary and the 28–30 s overlap.
    let starts = [1_000u32, 26_500, 62_000];
    let Some(clip) = speech_clip_at(tmp.path(), &starts, 75) else { return };
    let dir = models::model_dir(tmp.path());
    let path = models::ensure_model(&dir, Model::Base, &mut |_, _| {}, &AtomicBool::new(false)).unwrap();
    let mut seen = Vec::new();
    let progress = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let p = progress.clone();
    let t0 = std::time::Instant::now();
    let tr = transcribe_file(
        &path,
        &clip,
        &Options::default(),
        move |pct| p.borrow_mut().push(pct),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    eprintln!("TRANSCRIBE 75 s in windows in {:?}: {}", t0.elapsed(), tr.text());
    assert_eq!(tr.language, "en", "language detected once and kept");
    assert!((tr.duration_us - 75_000_000).abs() < 100_000, "{}", tr.duration_us);
    for w in tr.words.windows(2) {
        assert!(w[0].start_us <= w[1].start_us && w[0].end_us <= w[1].start_us + 1, "{w:?}");
    }
    // Each utterance comes through once, near where it was spoken: no word lost or doubled at the window seam.
    let words: Vec<String> = tr.words.iter().map(|w| norm(&w.text).join(" ")).collect();
    for (i, w) in words.iter().enumerate() {
        if w == "fox" {
            seen.push(tr.words[i].start_us);
        }
    }
    let timeline: Vec<String> = tr.words.iter().map(|w| format!("{}@{:.2}", w.text, w.start_us as f64 / 1e6)).collect();
    eprintln!("words: {}", timeline.join(" "));
    assert_eq!(seen.len(), 3, "'fox' at {seen:?}: {}", tr.text());
    // "fox" is the fourth word (~1 s in); whisper's DTW timing on synthetic speech can lag it by a few seconds even
    // inside one window, so the bound is loose. The seam itself must neither drop nor repeat words.
    for (at, start) in seen.iter().zip(starts) {
        let offset = at - i64::from(start) * 1000;
        assert!((0..4_000_000).contains(&offset), "'fox' at {at} for speech starting at {start} ms");
    }
    assert_eq!(words.iter().filter(|w| *w == "lazy").count(), 3, "{}", tr.text());
    let follows = tr.text().to_lowercase().matches("easier to follow").count();
    assert_eq!(follows, 3, "{}", tr.text());
    let progress = progress.borrow();
    assert!(progress.windows(2).all(|p| p[0] <= p[1]) && progress.iter().all(|p| (0..=100).contains(p)));
}
