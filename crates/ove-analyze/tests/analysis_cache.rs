//! The import analysis cache (tasks 2.1–2.3): one decode per file, cached peaks and loudness, re-analysis under a new
//! content key, recorded failures, and peak range queries against a brute-force reference. Needs the ffmpeg CLI to
//! make the media (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH); skipped with a message otherwise.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ove_analyze::analysis::{self, Status};
use ove_analyze::peaks::{BASE_BIN, PEAK_RATE, q_max, q_min};
use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};

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

/// A stereo 48 kHz WAV: left `amp`·sin(f), right half of it, with a 2 Hz amplitude wobble.
fn tone(dir: &Path, name: &str, freq: u32, amp: f64, secs: u32) -> PathBuf {
    let out = dir.join(name);
    let l = format!("{amp}*sin(2*PI*{freq}*t)*(0.6+0.4*sin(2*PI*2*t))");
    let src = format!("aevalsrc={l}|0.5*{l}:s=48000:d={secs}");
    let st = Command::new(ffmpeg().unwrap())
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &src, "-c:a", "pcm_s16le"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

fn decode_all(path: &Path) -> Vec<[f32; 2]> {
    ove_media::init();
    let spec = AudioOutputSpec { sample_rate: PEAK_RATE, channels: 2, format: SampleFormat::F32 };
    let mut d = AudioDecoder::open(path, spec).unwrap();
    let mut out = Vec::new();
    while let Some(c) = d.next_chunk().unwrap() {
        let Samples::F32(s) = c.samples else { panic!("f32 asked") };
        out.extend(s.chunks_exact(2).map(|f| [f[0], f[1]]));
    }
    out
}

#[test]
fn one_decode_serves_waveforms_and_loudness() {
    need_ffmpeg!();
    let dir = tempfile::tempdir().unwrap();
    let wav = tone(dir.path(), "a.wav", 440, 0.5, 10);
    let bundle = dir.path().join("bundle");
    let before = analysis::decodes_on_this_thread();
    // Import: the analysis pass.
    let t = Instant::now();
    let summary = analysis::ensure(&bundle, "k1", &wav).unwrap();
    let took = t.elapsed();
    eprintln!(
        "analysis of 10 s stereo: {took:?} ({:.0}x real time), peaks file {} bytes",
        10.0 / took.as_secs_f64(),
        std::fs::metadata(analysis::cache_paths(&bundle, "k1").peaks).unwrap().len()
    );
    // Three waveform zooms and one loudness query, all from the cache.
    for (a, b, bins) in [(0.0, 10.0, 200), (2.0, 4.0, 900), (5.0, 5.1, 300)] {
        let r = analysis::peak_range(&bundle, "k1", a, b, bins).unwrap();
        assert_eq!(r.bins, bins);
        assert!(r.data.iter().any(|v| *v != 0));
    }
    let Status::Ready(cached) = analysis::status(&bundle, "k1") else { panic!("not cached") };
    assert_eq!(cached, summary);
    assert_eq!(analysis::ensure(&bundle, "k1", &wav).unwrap(), summary);
    assert_eq!(analysis::decodes_on_this_thread() - before, 1, "the file is decoded exactly once");
    // The cached loudness equals an on-demand measurement.
    assert_eq!(summary.loudness, ove_analyze::loudness(&wav).unwrap());
    assert_eq!(summary.frames, 480_000);
    assert_eq!(summary.channels, 2);
}

#[test]
fn changed_content_is_analysed_again_and_failures_are_recorded() {
    need_ffmpeg!();
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    let wav = tone(dir.path(), "a.wav", 440, 0.5, 3);
    let before = analysis::decodes_on_this_thread();
    let first = analysis::ensure(&bundle, "content-1", &wav).unwrap();
    // New content at the same path: the caller's content id changes, so the old analysis is not used.
    tone(dir.path(), "a.wav", 440, 0.1, 3);
    let second = analysis::ensure(&bundle, "content-2", &wav).unwrap();
    assert_eq!(analysis::decodes_on_this_thread() - before, 2);
    let (l1, l2) = (first.loudness.integrated_lufs.unwrap(), second.loudness.integrated_lufs.unwrap());
    assert!((l1 - l2 - 13.98).abs() < 0.2, "{l1} vs {l2}");
    // Unreadable audio: an error is recorded, not retried, and does not affect other media.
    let junk = dir.path().join("junk.wav");
    std::fs::write(&junk, b"RIFF\0\0\0\0WAVEjunkjunkjunkjunk").unwrap();
    assert!(analysis::ensure(&bundle, "junk", &junk).is_err());
    assert!(matches!(analysis::status(&bundle, "junk"), Status::Failed(_)));
    let n = analysis::decodes_on_this_thread();
    assert!(analysis::ensure(&bundle, "junk", &junk).is_err());
    assert_eq!(analysis::decodes_on_this_thread(), n, "a failed file is not decoded again");
    assert!(matches!(analysis::status(&bundle, "content-2"), Status::Ready(_)));
    assert!(analysis::peak_range(&bundle, "junk", 0.0, 1.0, 10).is_err());
}

#[test]
fn peak_ranges_equal_a_brute_force_reference() {
    need_ffmpeg!();
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    let wav = tone(dir.path(), "a.wav", 97, 0.8, 6);
    analysis::ensure(&bundle, "k", &wav).unwrap();
    let samples = decode_all(&wav);
    for (a, b, bins) in [(0.0, 6.0, 500), (1.25, 1.5, 640), (0.0, 6.0, 7), (3.0, 3.001, 20)] {
        let r = analysis::peak_range(&bundle, "k", a, b, bins).unwrap();
        let size = (BASE_BIN << r.level) as f64;
        let (fa, fb) = (a * PEAK_RATE as f64, b * PEAK_RATE as f64);
        let span = (fb - fa) / bins as f64;
        // Coarsest sufficient: the next level's bins would be wider than a column.
        assert!(size <= span.max(BASE_BIN as f64) && (size * 2.0 > span || r.level == 0));
        for col in 0..bins {
            let x = fa + col as f64 * span;
            let lo_bin = (x / size).floor() as usize;
            let hi_bin = (((x + span) / size).ceil() as usize).max(lo_bin + 1);
            let s = &samples[(lo_bin as f64 * size) as usize..((hi_bin as f64 * size) as usize).min(samples.len())];
            for c in 0..2 {
                let mn = q_min(s.iter().map(|f| f[c]).fold(f32::MAX, f32::min));
                let mx = q_max(s.iter().map(|f| f[c]).fold(f32::MIN, f32::max));
                assert_eq!((r.data[(col * 2 + c) * 2], r.data[(col * 2 + c) * 2 + 1]), (mn, mx), "{a}..{b} col {col}");
            }
        }
    }
}
