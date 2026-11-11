//! Task 2.5: audio decode + resample, sample-accurate against the ffmpeg CLI (PCM diff ≤ 1 LSB).

mod common;

use std::process::Command;

use common::{AUDIO_FILES, media};
use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};

fn reference_s16(path: &std::path::Path, rate: u32) -> Vec<i16> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "s16le", "-ac", "2", "-ar", &rate.to_string(), "-"])
        .output()
        .expect("ffmpeg on PATH");
    assert!(out.status.success());
    out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

fn decode_s16(path: &std::path::Path, rate: u32) -> Vec<i16> {
    let spec = AudioOutputSpec { sample_rate: rate, channels: 2, format: SampleFormat::S16 };
    let mut d = AudioDecoder::open(path, spec).unwrap();
    let mut out = Vec::new();
    let mut expected_pts = 0i64;
    while let Some(c) = d.next_chunk().unwrap() {
        // Chunks are contiguous.
        assert!((c.pts_us - expected_pts).abs() <= 1, "gap at {} (expected {expected_pts})", c.pts_us);
        expected_pts = c.pts_us + c.frames as i64 * 1_000_000 / rate as i64;
        match c.samples {
            Samples::S16(v) => out.extend(v),
            Samples::F32(_) => unreachable!(),
        }
    }
    out
}

#[test]
fn pcm_matches_ffmpeg_within_one_lsb() {
    for rate in [48_000u32, 44_100] {
        for name in AUDIO_FILES {
            let path = media(name);
            let ours = decode_s16(&path, rate);
            let reference = reference_s16(&path, rate);
            let n = ours.len().min(reference.len());
            let len_diff = ours.len().abs_diff(reference.len()) / 2;
            assert!(
                len_diff <= 64,
                "{name}@{rate}: length {} vs {} sample frames",
                ours.len() / 2,
                reference.len() / 2
            );
            let max = ours[..n].iter().zip(&reference[..n]).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
            eprintln!("{name}@{rate}: {} frames, max diff {max} LSB, length diff {len_diff}", n / 2);
            assert!(max <= 1, "{name}@{rate}: max diff {max} LSB");
            // Not silence: the test tone is present.
            assert!(ours.iter().any(|s| s.abs() > 1000), "{name}: silent output");
        }
    }
}

#[test]
fn f32_output_matches_s16() {
    let path = media("audio_aac.m4a");
    let mut d = AudioDecoder::open(&path, AudioOutputSpec::default()).unwrap();
    let mut f = Vec::new();
    while let Some(c) = d.next_chunk().unwrap() {
        if let Samples::F32(v) = c.samples {
            f.extend(v);
        }
    }
    let s = decode_s16(&path, 48_000);
    assert_eq!(f.len(), s.len());
    let max = f.iter().zip(&s).map(|(a, b)| ((a * 32768.0).round() as i32 - *b as i32).abs()).max().unwrap();
    assert!(max <= 1, "{max}");
}

#[test]
fn audio_seek_lands_on_the_requested_sample() {
    for name in ["audio_pcm.wav", "audio_aac.m4a", "audio_opus.mka", "prores_hq.mov"] {
        let path = media(name);
        let spec = AudioOutputSpec { sample_rate: 48_000, channels: 2, format: SampleFormat::S16 };
        let total = decode_s16(&path, 48_000).len() / 2;
        let mut d = AudioDecoder::open(&path, spec).unwrap();
        d.seek(1_250_000).unwrap();
        let first = d.next_chunk().unwrap().unwrap();
        assert!((first.pts_us - 1_250_000).abs() <= 21, "{name}: first chunk at {}", first.pts_us);
        let mut n = first.frames;
        while let Some(c) = d.next_chunk().unwrap() {
            n += c.frames;
        }
        let expect = total - 60_000; // 1.25 s at 48 kHz
        assert!(n.abs_diff(expect) <= 1100, "{name}: {n} frames after seek, expected ≈ {expect}");
    }
}
