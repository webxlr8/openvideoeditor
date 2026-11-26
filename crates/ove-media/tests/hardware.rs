//! Task 2.4 on real hardware: the hardware decoder (D3D11VA on Windows, VideoToolbox on macOS) gives the same frames
//! as software decode, seeks frame-exactly, and a mid-stream hardware failure resumes in software at the same frame.
//!
//! Files whose codec the machine cannot decode in hardware open in software; they are listed and skipped. On a
//! machine without hardware decode (Linux, CI runners without a GPU) every file is skipped. `OVE_REQUIRE_HW=1` makes
//! the suite fail when no file used the hardware path, for runs on the reference machines.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod common;

use common::{Rng, VIDEO_FILES, frame_md5, media};
use ove_media::{DecodePath, DecoderOptions, FfmpegVideoDecoder, HwAccel, OutputFormat, VideoDecoder, VideoFrame};

fn opts(hwaccel: HwAccel) -> DecoderOptions {
    DecoderOptions { hwaccel, output: OutputFormat::Display, inject_hw_failure_after: None }
}

fn decode_all(d: &mut dyn VideoDecoder) -> Vec<VideoFrame> {
    let mut out = Vec::new();
    while let Some(f) = d.next_frame().unwrap() {
        out.push(f);
    }
    out
}

/// Software reference for a file: (pts, md5) of every frame in the display format.
fn software(name: &str) -> Vec<(i64, String)> {
    let mut d = FfmpegVideoDecoder::open(&media(name), opts(HwAccel::Off)).unwrap();
    decode_all(&mut d).iter().map(|f| (f.pts_us, frame_md5(f))).collect()
}

/// Opens `name` on the hardware path, or `None` when this machine decodes it in software. The device opens for any
/// codec it knows; whether it takes this stream (profile, chroma format, bit depth) shows on the first frame.
fn hardware(name: &str, inject: Option<u64>) -> Option<FfmpegVideoDecoder> {
    let o = DecoderOptions { inject_hw_failure_after: inject, ..opts(HwAccel::Auto) };
    let mut probe = FfmpegVideoDecoder::open(&media(name), o.clone()).ok()?;
    let first = probe.next_frame().ok()??;
    if !matches!(first.decode_path, DecodePath::Hardware(_)) {
        return None;
    }
    FfmpegVideoDecoder::open(&media(name), o).ok()
}

#[test]
fn hardware_frames_seeks_and_fallback_match_software() {
    if !common::ready() {
        return;
    }
    let (mut used, mut skipped) = (Vec::new(), Vec::new());
    for name in VIDEO_FILES {
        let Some(mut d) = hardware(name, None) else {
            skipped.push(*name);
            continue;
        };
        let api = d.decode_path().to_string();
        let reference = software(name);

        // Linear decode: same frames, same order, same pixels.
        let frames = decode_all(&mut d);
        assert_eq!(frames.len(), reference.len(), "{name} ({api}): frame count");
        for (i, (f, (pts, md5))) in frames.iter().zip(&reference).enumerate() {
            assert_eq!(f.pts_us, *pts, "{name} ({api}): frame {i} pts");
            assert_eq!(&frame_md5(f), md5, "{name} ({api}): frame {i} pixels");
        }

        // Random seeks land on the right frame with the right pixels.
        let mut d = hardware(name, None).unwrap();
        let mut rng = Rng(0xD3D1_1FA0 ^ name.len() as u64);
        for _ in 0..20 {
            let k = rng.below(reference.len() as u64) as usize;
            d.seek(reference[k].0).unwrap();
            let f = d.next_frame().unwrap().unwrap();
            assert_eq!((f.pts_us, frame_md5(&f)), reference[k].clone(), "{name} ({api}): seek to frame {k}");
        }

        // Mid-stream hardware failure: open_video's fallback continues in software at the same frame.
        let k = (reference.len() / 3) as u64;
        let o = DecoderOptions { inject_hw_failure_after: Some(k), ..opts(HwAccel::Auto) };
        let mut d = ove_media::open_video(&media(name), o).unwrap();
        let frames = decode_all(&mut d);
        let got: Vec<(i64, String)> = frames.iter().map(|f| (f.pts_us, frame_md5(f))).collect();
        assert_eq!(got, reference, "{name} ({api}): frames across the fallback at frame {k}");
        assert_eq!(d.fallback_count(), 1, "{name} ({api}): one fallback");
        assert!(matches!(frames[0].decode_path, DecodePath::Hardware(_)), "{name}: starts on hardware");
        assert_eq!(frames.last().unwrap().decode_path, DecodePath::Software, "{name}: ends in software");

        used.push(format!("{name} ({api}, {} frames)", reference.len()));
    }
    eprintln!("hardware path: {used:?}\nsoftware only on this machine: {skipped:?}");
    if std::env::var("OVE_REQUIRE_HW").is_ok_and(|v| v == "1") {
        assert!(!used.is_empty(), "OVE_REQUIRE_HW=1 but no file decoded on the hardware path");
    }
}
