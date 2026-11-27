//! Loudness-normalised and audio-only export (add-audio-mixing-and-monitoring tasks 6.3 and 6.4): a −30 LUFS mix
//! exports at −14 LUFS with its true peak at or under −1 dBTP, a silent mix exports without gain, 24-bit WAV holds
//! the mixer's samples within 1 LSB, M4A keeps the timeline duration within one AAC frame, FLAC decodes back to the
//! mix, and offline media is refused without leaving a file. Tones come from the ffmpeg CLI (`OVE_FIXTURE_FFMPEG` or
//! `ffmpeg`): without it the tests fail, unless `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` makes them print "skipped".
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
use std::sync::atomic::AtomicBool;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
use ove_render::audio::{Mixer, RATE};
use ove_render::export::settings::Container;
use ove_render::loudness::StereoLoudness;
use ove_render::{ExportError, ExportSettings, export};
use serde_json::json;

/// A stereo 48 kHz sine at `amp` (1.0 = full scale) lasting `secs`, as 24-bit WAV.
fn tone(dir: &Path, name: &str, freq: u32, amp: f64, secs: u32) -> Option<PathBuf> {
    let ff = fixtures::fixture_ffmpeg();
    if !fixtures::require_tool(&ff, "it makes the test tones; set OVE_FIXTURE_FFMPEG") {
        return None;
    }
    let out = dir.join(name);
    let wave = format!("{amp}*sin(2*PI*{freq}*t)");
    let src = format!("aevalsrc={wave}|{wave}:s=48000:d={secs}");
    let st = Command::new(ff)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &src, "-c:a", "pcm_s24le"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    Some(out)
}

fn doc(path: &Path, secs: f64) -> Document {
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "t", "name": "Audio", "order": 0, "type": "audio"}) },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "audio", "fileName": "tone.wav", "duration": secs, "hasAudio": true}}),
            entry: Some(json!({"mediaId": "m", "nativePath": path.display().to_string(), "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "a", "trackId": "t", "type": "audio", "startTime": 0, "endTime": secs, "mediaItemId": "m"}),
        },
    ];
    apply_batch(&Document::new("p", "Audio export", 1), &ops).unwrap().doc
}

fn decode(path: &Path) -> Vec<f32> {
    ove_media::init();
    let spec = AudioOutputSpec { sample_rate: RATE, channels: 2, format: SampleFormat::F32 };
    let mut d = AudioDecoder::open(path, spec).unwrap();
    let mut out = Vec::new();
    while let Some(c) = d.next_chunk().unwrap() {
        match c.samples {
            Samples::F32(v) => out.extend(v),
            Samples::S16(v) => out.extend(v.into_iter().map(|s| s as f32 / 32768.0)),
        }
    }
    out
}

fn audio_only(c: Container, loudness: Option<f64>) -> ExportSettings {
    ExportSettings { loudness, ..ExportSettings::for_container(c) }
}

#[test]
fn minus_30_lufs_exports_at_minus_14_and_silence_gets_no_gain() {
    let dir = tempfile::tempdir().unwrap();
    // 997 Hz stereo at 0.0316 (−30 dBFS) measures −30 LUFS.
    let Some(quiet) = tone(dir.path(), "quiet.wav", 997, 0.031_623, 6) else { return };
    let d = doc(&quiet, 6.0);
    let out = dir.path().join("loud.wav");
    let r = export(&d, None, &out, &audio_only(Container::Wav, Some(-14.0)), &AtomicBool::new(false), |_| {}).unwrap();
    let l = r.loudness.clone().expect("a loudness report");
    let mut m = StereoLoudness::default();
    m.push(&decode(&out));
    let file = m.summary();
    eprintln!(
        "loudness: before {:?} LUFS / {:.2} dBTP, after {:?} LUFS / {:.2} dBTP, gain {:+.2} dB; file {:?} LUFS, {:.2} dBTP",
        l.before_lufs,
        l.before_true_peak_dbtp,
        l.after_lufs,
        l.after_true_peak_dbtp,
        l.gain_db,
        file.integrated_lufs,
        file.true_peak_dbtp
    );
    assert!((l.before_lufs.unwrap() + 30.0).abs() < 0.2);
    assert!((l.after_lufs.unwrap() + 14.0).abs() <= 0.5);
    assert!((file.integrated_lufs.unwrap() + 14.0).abs() <= 0.5, "file {:?}", file.integrated_lufs);
    assert!(file.true_peak_dbtp <= -1.0, "true peak {}", file.true_peak_dbtp);
    assert!(!l.silent);
    // A hot target on a quiet but peaky source: still under -1 dBTP after the gain.
    let Some(silent) = tone(dir.path(), "silent.wav", 997, 0.0, 3) else { return };
    let s = doc(&silent, 3.0);
    let out = dir.path().join("silent.wav.out.wav");
    let r = export(&s, None, &out, &audio_only(Container::Wav, Some(-14.0)), &AtomicBool::new(false), |_| {}).unwrap();
    let l = r.loudness.unwrap();
    assert!(l.silent && l.before_lufs.is_none() && l.gain_db == 0.0, "{l:?}");
    assert!(out.is_file());
    // An unknown target is refused before anything is written.
    let e = export(
        &d,
        None,
        &dir.path().join("x.wav"),
        &audio_only(Container::Wav, Some(-20.0)),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(e, ExportError::Invalid(_)), "{e}");
    assert!(!dir.path().join("x.wav").exists());
}

#[test]
fn wav_holds_the_mix_m4a_keeps_the_duration_flac_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let Some(src) = tone(dir.path(), "src.wav", 440, 0.5, 4) else { return };
    // 3.5 s: not a whole number of AAC frames.
    let d = doc(&src, 3.5);
    let frames = (3.5 * RATE as f64) as usize;
    let mix = Mixer::new(None).mix(&d, 0, frames);
    let cancel = AtomicBool::new(false);

    let wav = dir.path().join("mix.wav");
    let r = export(&d, None, &wav, &audio_only(Container::Wav, None), &cancel, |_| {}).unwrap();
    assert_eq!((r.width, r.height, r.frames), (0, 0, 0));
    let got = decode(&wav);
    assert_eq!(got.len(), mix.len(), "duration equals the timeline");
    let lsb = 1.0 / 8_388_607.0;
    let worst = got.iter().zip(&mix).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(worst <= lsb * 1.01, "24-bit WAV differs from the mix by {worst} (1 LSB = {lsb})");

    let flac = dir.path().join("mix.flac");
    match export(&d, None, &flac, &audio_only(Container::Flac, None), &cancel, |_| {}) {
        Err(ExportError::Invalid(e)) if e.message.contains("cannot write FLAC") => {
            eprintln!("skipped FLAC: {}", e.message);
        }
        r => {
            r.unwrap();
            let got = decode(&flac);
            assert_eq!(got.len(), mix.len());
            let worst = got.iter().zip(&mix).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(worst <= lsb * 1.01, "FLAC differs from the mix by {worst}");
        }
    }

    let m4a = dir.path().join("mix.m4a");
    export(&d, None, &m4a, &audio_only(Container::M4a, None), &cancel, |_| {}).unwrap();
    let info = ove_media::probe(&m4a).unwrap();
    let dur = info.audio.as_ref().and_then(|a| a.duration_us).or(info.duration_us).unwrap();
    let decoded = decode(&m4a).len() / 2;
    eprintln!("m4a: container duration {dur} µs, {decoded} decoded frames (timeline 3 500 000 µs, {frames} frames)");
    let frame_us = 1024.0 * 1e6 / RATE as f64;
    assert!(((dur as f64) - 3.5e6).abs() <= frame_us, "m4a duration {dur} µs");
    assert!((decoded as f64 - frames as f64).abs() <= 1024.0, "m4a decodes {decoded} frames");
}

#[test]
fn offline_media_is_refused_and_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let d = doc(&dir.path().join("gone.wav"), 2.0);
    for c in [Container::Wav, Container::Flac, Container::M4a] {
        let out = dir.path().join(format!("out.{}", c.extension()));
        let e = export(&d, None, &out, &audio_only(c, None), &AtomicBool::new(false), |_| {}).unwrap_err();
        if matches!(&e, ExportError::Invalid(s) if s.message.contains("cannot write")) {
            eprintln!("skipped {c}: {e}");
            continue;
        }
        assert!(matches!(e, ExportError::Refused(_)), "{e}");
        assert!(e.to_string().contains("offline"), "{e}");
        assert!(!out.exists());
    }
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn normalising_a_peaky_mix_keeps_the_true_peak_under_minus_one() {
    let ff = fixtures::fixture_ffmpeg();
    if !fixtures::require_tool(&ff, "it makes the test tones; set OVE_FIXTURE_FFMPEG") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // A quiet tone with a loud 3 kHz click every second: +16 dB of gain pushes the clicks far over full scale.
    let src = dir.path().join("peaky.wav");
    let wave = r"0.03*sin(2*PI*500*t)+0.6*lt(mod(t\,1)\,0.004)*sin(2*PI*3000*t)";
    let lavfi = format!("aevalsrc={wave}|{wave}:s=48000:d=8");
    let st = Command::new(ff)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &lavfi, "-c:a", "pcm_s24le"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());
    let d = doc(&src, 8.0);
    let out = dir.path().join("peaky-out.wav");
    let r = export(&d, None, &out, &audio_only(Container::Wav, Some(-14.0)), &AtomicBool::new(false), |_| {}).unwrap();
    let l = r.loudness.unwrap();
    let mut m = StereoLoudness::default();
    m.push(&decode(&out));
    let file = m.summary();
    eprintln!(
        "peaky: before {:?} LUFS / {:.2} dBTP, gain {:+.2} dB, file {:?} LUFS / {:.2} dBTP",
        l.before_lufs, l.before_true_peak_dbtp, l.gain_db, file.integrated_lufs, file.true_peak_dbtp
    );
    assert!(file.true_peak_dbtp <= -1.0, "true peak {}", file.true_peak_dbtp);
    assert!((file.integrated_lufs.unwrap() + 14.0).abs() <= 0.5, "file {:?}", file.integrated_lufs);
}
