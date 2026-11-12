//! Mixdown quality: the peak limiter keeps loud sums under -1 dBFS, and speed changes resample without aliasing.
//! Tones are made with the ffmpeg CLI (`OVE_FIXTURE_FFMPEG` or `ffmpeg`): without it the tests fail, unless
//! `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` makes them print "skipped".
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

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::audio::{CEILING, CHANNELS, Mixer, RATE};
use serde_json::json;

/// A stereo 48 kHz PCM sine of `amp` (1.0 = full scale) lasting `secs`.
fn tone(dir: &Path, freq: u32, amp: f32, secs: u32) -> Option<PathBuf> {
    let ff = fixtures::fixture_ffmpeg();
    if !fixtures::require_tool(&ff, "it makes the test tones; set OVE_FIXTURE_FFMPEG") {
        return None;
    }
    let out = dir.join(format!("sine_{freq}_{amp}.wav"));
    // Two channel expressions: a mono source upmixed with -ac 2 would lose 3 dB to the centre mix level.
    let wave = format!("{amp}*sin(2*PI*{freq}*t)");
    let src = format!("aevalsrc={wave}|{wave}:s=48000:d={secs}");
    let st = Command::new(ff)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &src, "-c:a", "pcm_s16le"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg failed to make {}", out.display());
    Some(out)
}

/// Each clip on its own track from 0 to `secs`, with the given playback speed.
fn doc(clips: &[(&Path, f64)], secs: f64) -> Document {
    let mut ops = vec![Op::SetAspectRatio { aspect_ratio: "16:9".into() }];
    for (i, (path, speed)) in clips.iter().enumerate() {
        let (track, media) = (format!("t{i}"), format!("m{i}"));
        ops.push(Op::AddTrack { track: json!({"id": track, "name": track, "order": i, "type": "audio"}) });
        ops.push(Op::AddMedia {
            media: json!({"id": media, "data": {"type": "audio", "fileName": "tone.wav", "duration": 10, "hasAudio": true}}),
            entry: Some(json!({"mediaId": media, "nativePath": path.display().to_string(), "ownedByBundle": false})),
        });
        ops.push(Op::AddItem {
            item: json!({"id": format!("a{i}"), "trackId": track, "type": "audio", "startTime": 0, "endTime": secs,
                "mediaItemId": media, "playbackSpeed": speed}),
        });
    }
    apply_batch(&Document::new("p", "Mix", 1), &ops).unwrap().doc
}

/// Mixes `secs` from `t0` in 1024-frame blocks, as playback does; returns the left channel.
fn mix_left(doc: &Document, t0_us: i64, secs: f64) -> Vec<f32> {
    let mut m = Mixer::new(None);
    let total = (secs * RATE as f64) as usize;
    let mut out = Vec::with_capacity(total * CHANNELS);
    let mut t = t0_us;
    while out.len() < total * CHANNELS {
        out.extend(m.mix(doc, t, 1024));
        t += 1024 * 1_000_000 / RATE as i64;
    }
    out.chunks(CHANNELS).take(total).map(|f| f[0]).collect()
}

/// Share of the signal's energy within ±`width` Hz of `freq` (Hann-windowed DFT at the bins concerned).
fn energy_share(x: &[f32], freq: f64, width: f64) -> f64 {
    let n = x.len();
    let w: Vec<f64> = (0..n)
        .map(|i| {
            let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            f64::from(x[i]) * hann
        })
        .collect();
    let total: f64 = w.iter().map(|v| v * v).sum::<f64>() * n as f64 / 2.0;
    let bin_hz = RATE as f64 / n as f64;
    let (lo, hi) = (((freq - width) / bin_hz).floor() as usize, ((freq + width) / bin_hz).ceil() as usize);
    let mut band = 0.0;
    for k in lo..=hi {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in w.iter().enumerate() {
            let ph = 2.0 * std::f64::consts::PI * k as f64 * i as f64 / n as f64;
            re += v * ph.cos();
            im -= v * ph.sin();
        }
        band += re * re + im * im;
    }
    band / total
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|s| s * s).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

#[test]
fn two_full_scale_sines_never_exceed_minus_one_dbfs() {
    let dir = tempfile::tempdir().unwrap();
    let Some(sine) = tone(dir.path(), 1000, 0.999, 2) else { return };
    let d = doc(&[(&sine, 1.0), (&sine, 1.0)], 2.0);
    let mut m = Mixer::new(None);
    let mut peak = 0f32;
    let mut all = Vec::new();
    for b in 0..80 {
        let block = m.mix(&d, b * 1024 * 1_000_000 / RATE as i64, 1024);
        peak = block.iter().fold(peak, |p, s| p.max(s.abs()));
        all.extend(block);
    }
    eprintln!("summed full-scale sines: peak {peak:.4} ({:.2} dBFS), rms {:.3}", 20.0 * peak.log10(), rms(&all));
    assert!(peak <= CEILING, "peak {peak} over -1 dBFS");
    // Limited, not crushed: a sine with its peak at -1 dBFS has an RMS of 0.63.
    assert!(rms(&all) > 0.55, "rms {}", rms(&all));
}

#[test]
fn speed_changes_shift_the_tone_without_aliasing_or_spurs() {
    let dir = tempfile::tempdir().unwrap();
    let Some(k1) = tone(dir.path(), 1000, 0.5, 4) else { return };
    let Some(k15) = tone(dir.path(), 15000, 0.5, 4) else { return };
    // 2×: a 1 kHz tone plays at 2 kHz, with (almost) all its energy there.
    let x = mix_left(&doc(&[(&k1, 2.0)], 1.5), 100_000, 0.5);
    let share2 = energy_share(&x, 2000.0, 30.0);
    // 1.5×: a fractional ratio, where nearest-sample picking used to add jitter spurs.
    let y = mix_left(&doc(&[(&k1, 1.5)], 1.5), 100_000, 0.5);
    let share15 = energy_share(&y, 1500.0, 30.0);
    // 2× of 15 kHz is 30 kHz, above the 24 kHz Nyquist: it must be filtered out, not folded down to 18 kHz.
    let z = mix_left(&doc(&[(&k15, 2.0)], 1.5), 100_000, 0.5);
    eprintln!(
        "2x 1 kHz: {:.5} of energy at 2 kHz (rms {:.3}); 1.5x: {:.5} at 1.5 kHz; 2x 15 kHz: rms {:.5} (input 0.354)",
        share2,
        rms(&x),
        share15,
        rms(&z)
    );
    assert!(rms(&x) > 0.3, "2x tone keeps its level: {}", rms(&x));
    assert!(share2 > 0.999, "2x: {share2}");
    assert!(share15 > 0.999, "1.5x: {share15}");
    assert!(rms(&z) < 0.0035, "aliasing at 2x: rms {} (> -40 dB of the input)", rms(&z));
}

#[test]
fn speed_blocks_join_seamlessly() {
    let dir = tempfile::tempdir().unwrap();
    let Some(k1) = tone(dir.path(), 1000, 0.5, 4) else { return };
    let d = doc(&[(&k1, 1.5)], 2.0);
    let mut a = Mixer::new(None);
    let mut joined = a.mix(&d, 200_000, 1024);
    joined.extend(a.mix(&d, 200_000 + 1024 * 1_000_000 / RATE as i64, 1024));
    let one = Mixer::new(None).mix(&d, 200_000, 2048);
    let diff = joined.iter().zip(&one).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
    assert!(diff < 1e-6, "block joins differ by {diff}");
}
