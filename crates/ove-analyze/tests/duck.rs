//! Ducking on a synthetic dialogue fixture (tasks 5.1, 5.2): speech from the level gate and from transcript word
//! timings, and the mixed music level during and away from speech. Needs the ffmpeg CLI to make the media
//! (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH); skipped with a message otherwise.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ove_analyze::duck::{self, DuckError, DuckParams, Mode, SpeechSource};
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::audio::{CHANNELS, Mixer, RATE};
use serde_json::json;

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

macro_rules! need_ffmpeg {
    () => {
        if ffmpeg().is_none() {
            eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
            return;
        }
    };
}

fn wav(dir: &Path, name: &str, expr: &str, secs: u32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("aevalsrc={expr}|{expr}:s=48000:d={secs}");
    let st = Command::new(ffmpeg().unwrap())
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &src, "-c:a", "pcm_s16le"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

/// Steady music for 12 s, and "dialogue": two lines (2–4 s and 6–7.5 s) of a 1 kHz voice-like tone, silent between.
fn fixture(dir: &Path) -> Document {
    let music = wav(dir, "music.wav", "0.2*sin(2*PI*440*t)", 12);
    let dialogue = wav(dir, "dialogue.wav", "0.3*sin(2*PI*1000*t)*(between(t\\,2\\,4)+between(t\\,6\\,7.5))", 12);
    let mut ops = vec![Op::SetAspectRatio { aspect_ratio: "16:9".into() }];
    for (i, (id, path, track)) in [("dlg", &dialogue, "Dialogue"), ("mus", &music, "Music")].iter().enumerate() {
        ops.push(Op::AddTrack { track: json!({"id": format!("t_{id}"), "name": track, "order": i, "type": "audio"}) });
        ops.push(Op::AddMedia {
            media: json!({"id": format!("m_{id}"), "data": {"type": "audio", "fileName": format!("{id}.wav"), "duration": 12}}),
            entry: Some(json!({"mediaId": format!("m_{id}"), "nativePath": path.display().to_string(), "ownedByBundle": false,
                "contentId": format!("{id}0000000000000000000000000000")})),
        });
        ops.push(Op::AddItem {
            item: json!({"id": id, "trackId": format!("t_{id}"), "type": "audio", "startTime": 0, "endTime": 12, "mediaItemId": format!("m_{id}")}),
        });
    }
    apply_batch(&Document::new("p", "Duck", 1), &ops).unwrap().doc
}

/// RMS level (dB) of the music alone in 50 ms windows.
fn music_levels(doc: &Document) -> Vec<f64> {
    let mut solo = doc.clone();
    let dlg = (**solo.project.items.get("dlg").unwrap()).clone();
    let mut d = dlg;
    d.fields.insert("muted".into(), json!(true));
    solo.project.items.insert("dlg".into(), std::sync::Arc::new(d));
    let mut m = Mixer::new(None);
    let win = (RATE / 20) as usize;
    (0..240)
        .map(|i| {
            let s = m.mix(&solo, i * 50_000, win);
            let ms = s.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / (win * CHANNELS) as f64;
            10.0 * ms.log10()
        })
        .collect()
}

fn at(levels: &[f64], t: f64) -> f64 {
    levels[(t * 20.0) as usize]
}

#[test]
fn level_gate_finds_the_lines_and_the_music_ducks_12_db() {
    need_ffmpeg!();
    let dir = tempfile::tempdir().unwrap();
    let doc = fixture(dir.path());
    let p = DuckParams::default();
    let plan =
        duck::plan(&doc, Some(dir.path()), &["Music".into()], &["dlg".into()], &p, SpeechSource::Level, Mode::Replace)
            .unwrap();
    assert_eq!(plan.source, "level");
    assert_eq!(plan.speech.len(), 2, "{:?}", plan.speech);
    for ((a, b), (wa, wb)) in plan.speech.iter().zip([(2.0, 4.0), (6.0, 7.5)]) {
        assert!((a - wa).abs() <= 0.01 && (b - wb).abs() <= 0.01, "span {a}–{b}, want {wa}–{wb}");
    }
    // Ramp corners within 10 ms: down from start - attack, back up by end + release.
    let keys: Vec<f64> = plan.items[0].keyframes.iter().map(|k| k.0).collect();
    for want in [1.8, 2.0, 4.0, 4.4, 5.8, 6.0, 7.5, 7.9] {
        assert!(keys.iter().any(|t| (t - want).abs() <= 0.01), "no keyframe within 10 ms of {want}: {keys:?}");
    }
    let before = music_levels(&doc);
    let after = music_levels(&apply_batch(&doc, &plan.ops).unwrap().doc);
    for t in [2.2, 3.0, 3.8, 6.2, 7.0, 7.3] {
        let d = at(&after, t) - at(&before, t);
        assert!((d + 12.0).abs() <= 0.5, "during speech at {t}: {d:.2} dB");
    }
    for t in [0.5, 1.5, 4.6, 5.5, 8.2, 11.0] {
        let d = at(&after, t) - at(&before, t);
        assert!(d.abs() <= 0.2, "away from speech at {t}: {d:.2} dB");
    }
    eprintln!(
        "ducked music: {:.2} dB during speech, {:.2} dB away",
        at(&after, 3.0) - at(&before, 3.0),
        at(&after, 5.0) - at(&before, 5.0)
    );
}

#[test]
fn transcript_words_are_used_when_cached() {
    need_ffmpeg!();
    let dir = tempfile::tempdir().unwrap();
    let mut doc = fixture(dir.path());
    // The dialogue clip starts 1 s into its source and plays at 2×: source word times map through trim and speed.
    let mut set = serde_json::Map::new();
    set.insert("trimStart".into(), json!(1));
    set.insert("playbackSpeed".into(), json!(2));
    set.insert("endTime".into(), json!(5.5));
    doc = apply_batch(&doc, &[Op::UpdateItem { id: "dlg".into(), set }]).unwrap().doc;
    let tdir = dir.path().join("cache").join("transcripts");
    std::fs::create_dir_all(&tdir).unwrap();
    let words = json!({"language": "en", "model": "base", "duration_us": 12_000_000, "words": [
        {"text": "Hello", "start_us": 2_000_000, "end_us": 2_600_000, "confidence": 0.9},
        {"text": "there", "start_us": 2_700_000, "end_us": 3_200_000, "confidence": 0.9},
        {"text": "Again", "start_us": 7_000_000, "end_us": 8_000_000, "confidence": 0.9}]});
    std::fs::write(tdir.join("dlg0000000000000000000000000000-base-en.json"), words.to_string()).unwrap();
    let p = DuckParams::default();
    let plan = duck::plan(
        &doc,
        Some(dir.path()),
        &["mus".into()],
        &["Dialogue".into()],
        &p,
        SpeechSource::Auto,
        Mode::Replace,
    )
    .unwrap();
    assert_eq!(plan.source, "transcript");
    // Source 2.0–3.2 s → timeline 0.5–1.1 s (merged within the 0.5 s hold); 7–8 s → 3.0–3.5 s.
    assert_eq!(plan.speech, vec![(0.5, 1.1), (3.0, 3.5)]);
    // Forcing the level source ignores the transcript.
    let lvl =
        duck::plan(&doc, Some(dir.path()), &["mus".into()], &["dlg".into()], &p, SpeechSource::Level, Mode::Replace)
            .unwrap();
    assert_eq!(lvl.source, "level");
}

#[test]
fn refusals_name_the_problem() {
    need_ffmpeg!();
    let dir = tempfile::tempdir().unwrap();
    let doc = fixture(dir.path());
    let p = DuckParams::default();
    let e =
        duck::plan(&doc, None, &["nope".into()], &["dlg".into()], &p, SpeechSource::Level, Mode::Replace).unwrap_err();
    assert_eq!(e, DuckError::UnknownItem("nope".into()));
    assert!(e.to_string().contains("nope"));
    // Dialogue that is silent: no speech, no plan.
    let mut quiet = doc.clone();
    let mut set = serde_json::Map::new();
    set.insert("volume".into(), json!(0));
    quiet = apply_batch(&quiet, &[Op::UpdateItem { id: "dlg".into(), set }]).unwrap().doc;
    let e =
        duck::plan(&quiet, None, &["mus".into()], &["dlg".into()], &p, SpeechSource::Level, Mode::Replace).unwrap_err();
    assert_eq!(e, DuckError::NoSpeech);
    let e = duck::plan(
        &doc,
        Some(dir.path()),
        &["mus".into()],
        &["dlg".into()],
        &p,
        SpeechSource::Transcript,
        Mode::Replace,
    )
    .unwrap_err();
    assert_eq!(e, DuckError::NoTranscript);
}
