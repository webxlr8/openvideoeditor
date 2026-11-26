//! Voice clean-up on generated audio: noise between words drops, the speech stays, and the output keeps the source's
//! length and timing.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::sync::atomic::AtomicBool;

const RATE: usize = 48_000;

/// A mono 16-bit WAV.
fn write_wav(path: &Path, samples: &[f32]) {
    let data: Vec<u8> = samples.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | (1 << 16), RATE as u32, RATE as u32 * 2, 2 | (16 << 16)] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out).unwrap();
}

fn read_wav(path: &Path) -> (u16, Vec<f32>) {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[0..4], b"RIFF");
    let channels = u16::from_le_bytes([b[22], b[23]]);
    assert_eq!(u32::from_le_bytes([b[24], b[25], b[26], b[27]]), RATE as u32);
    let len = u32::from_le_bytes([b[40], b[41], b[42], b[43]]) as usize;
    assert_eq!(b.len(), 44 + len, "the header's size matches the data");
    (channels, b[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0).collect())
}

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
}

/// Deterministic white noise.
fn noise(n: usize, amp: f32) -> Vec<f32> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

/// Something voice-like: a 140 Hz buzz with harmonics, its pitch and loudness wobbling like syllables.
fn voice(n: usize) -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let f0 = 140.0 + 20.0 * (t * 3.0 * std::f32::consts::TAU).sin();
            phase += f0 / RATE as f32 * std::f32::consts::TAU;
            let env = 0.5 + 0.5 * (t * 4.0 * std::f32::consts::TAU).sin().abs();
            let s: f32 = (1..12).map(|h| (phase * h as f32).sin() / h as f32).sum();
            0.25 * env * s
        })
        .collect()
}

#[test]
fn noise_drops_speech_stays_and_timing_is_kept() {
    let d = tempfile::tempdir().unwrap();
    // 1 s of noise alone, then 2 s of voice over the same noise, then 0.5 s of noise; an odd length on purpose.
    let n = RATE * 7 / 2 + 123;
    let hiss = noise(n, 0.08);
    let talk = voice(RATE * 2);
    let src: Vec<f32> =
        (0..n).map(|i| hiss[i] + if (RATE..RATE * 3).contains(&i) { talk[i - RATE] } else { 0.0 }).collect();
    let input = d.path().join("noisy.wav");
    write_wav(&input, &src);
    let output = d.path().join("clean.wav");
    let mut last = 0.0;
    let c = ove_analyze::clean_voice(&input, &output, &AtomicBool::new(false), &mut |p| last = p).unwrap();
    assert_eq!(last, 1.0);
    assert_eq!(c.channels, 1);
    let (channels, out) = read_wav(&output);
    assert_eq!(channels, 1);
    assert_eq!(out.len(), n, "same length as the source");
    assert!((c.seconds - n as f64 / RATE as f64).abs() < 1e-9);
    assert!(!d.path().join("clean.wav.part").exists());

    let quiet = RATE / 4..RATE * 3 / 4;
    let (before, after) = (rms(&src[quiet.clone()]), rms(&out[quiet]));
    // Noise before the first word too (the network is primed on the start of the file).
    assert!(after < before * 0.1, "noise alone: {before:.4} -> {after:.4}");
    let talking = RATE * 3 / 2..RATE * 5 / 2;
    let (before, after) = (rms(&talk[RATE / 2..RATE * 3 / 2]), rms(&out[talking.clone()]));
    assert!(after > before * 0.4, "the voice stays: {before:.4} -> {after:.4}");

    // Aligned: the cleaned voice lines up with the clean voice (best lag within a few samples).
    let clean = &talk[RATE / 2..RATE * 3 / 2];
    let got = &out[talking];
    let score =
        |lag: i64| -> f32 { (2000..clean.len() - 2000).map(|i| clean[i] * got[(i as i64 + lag) as usize]).sum() };
    let best = (-600..=600).max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap();
    assert!(best.abs() <= 2, "cleaned audio is {best} samples off");
}

#[test]
fn cancelling_leaves_no_file() {
    let d = tempfile::tempdir().unwrap();
    let input = d.path().join("a.wav");
    write_wav(&input, &noise(RATE, 0.1));
    let output = d.path().join("b.wav");
    assert!(ove_analyze::clean_voice(&input, &output, &AtomicBool::new(true), &mut |_| {}).is_err());
    assert!(!output.exists() && !d.path().join("b.wav.part").exists());
}
