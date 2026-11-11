//! Tasks 2.3 / 2.4: frame-exact software decode and seek, verified by per-frame md5 against `ffmpeg -f framemd5`;
//! hardware fallback (at open, unsupported stream, mid-stream via fault injection).

mod common;

use std::collections::BTreeMap;

use common::{Rng, VIDEO_FILES, frame_md5, media, reference_framemd5};
use ove_media::{
    DecodePath, DecoderOptions, FallbackDecoder, FfmpegVideoDecoder, HwAccel, MediaError, OutputFormat, PixelFormat,
    VideoDecoder, VideoFrame, open_video,
};

fn native() -> DecoderOptions {
    DecoderOptions { hwaccel: HwAccel::Off, output: OutputFormat::Native, inject_hw_failure_after: None }
}

fn decode_all(d: &mut dyn VideoDecoder) -> Vec<VideoFrame> {
    let mut out = Vec::new();
    while let Some(f) = d.next_frame().unwrap() {
        out.push(f);
    }
    out
}

/// Seek targets per file; `OVE_SEEK_TARGETS=200` runs the full suite from the task list.
fn seek_targets() -> usize {
    std::env::var("OVE_SEEK_TARGETS").ok().and_then(|v| v.parse().ok()).unwrap_or(40)
}

#[test]
fn linear_decode_matches_framemd5_for_every_video_file() {
    for name in VIDEO_FILES {
        let path = media(name);
        let reference = reference_framemd5(&path);
        let mut d = FfmpegVideoDecoder::open(&path, native()).unwrap();
        let frames = decode_all(&mut d);
        assert_eq!(frames.len(), reference.len(), "{name}: frame count");
        for (i, (f, r)) in frames.iter().zip(&reference).enumerate() {
            assert_eq!(&frame_md5(f), r, "{name}: frame {i} (pts {} µs)", f.pts_us);
        }
        // Presentation order: strictly increasing pts.
        assert!(frames.windows(2).all(|w| w[0].pts_us < w[1].pts_us), "{name}: pts order");
        assert_eq!(frames[0].decode_path, DecodePath::Software);
    }
}

#[test]
fn random_seeks_are_frame_exact() {
    let n = seek_targets();
    let mut timings = BTreeMap::new();
    for name in VIDEO_FILES {
        let path = media(name);
        let reference = reference_framemd5(&path);
        let mut lin = FfmpegVideoDecoder::open(&path, native()).unwrap();
        let pts: Vec<i64> = decode_all(&mut lin).iter().map(|f| f.pts_us).collect();
        let mut d = FfmpegVideoDecoder::open(&path, native()).unwrap();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ name.len() as u64);
        let mut worst = 0f64;
        for i in 0..n {
            let k = rng.below(pts.len() as u64) as usize;
            // Half the targets hit the frame's pts exactly, half land inside its presentation interval.
            let t = if i % 2 == 0 || k + 1 == pts.len() {
                pts[k]
            } else {
                pts[k] + rng.below((pts[k + 1] - pts[k]) as u64) as i64
            };
            let start = std::time::Instant::now();
            d.seek(t).unwrap();
            let f = d.next_frame().unwrap().unwrap_or_else(|| panic!("{name}: no frame after seek to {t}"));
            worst = worst.max(start.elapsed().as_secs_f64() * 1e3);
            assert_eq!(f.pts_us, pts[k], "{name}: seek to {t} µs should show frame {k}");
            assert_eq!(frame_md5(&f), reference[k], "{name}: seek to frame {k} content");
            // The frames after a seek continue in order.
            if k + 1 < pts.len() {
                let g = d.next_frame().unwrap().unwrap();
                assert_eq!(g.pts_us, pts[k + 1], "{name}: frame after {k}");
                assert_eq!(frame_md5(&g), reference[k + 1]);
            }
        }
        // Before the first frame → first frame; past the end → last frame.
        d.seek(-5_000).unwrap();
        assert_eq!(d.next_frame().unwrap().unwrap().pts_us, pts[0]);
        d.seek(pts[pts.len() - 1] + 10_000_000).unwrap();
        assert_eq!(d.next_frame().unwrap().unwrap().pts_us, *pts.last().unwrap());
        timings.insert(*name, worst);
    }
    eprintln!("worst seek-to-frame (ms, {n} targets/file): {timings:?}");
}

#[test]
fn frame_step_advances_exactly() {
    // Spec scenario "Frame step": 10 steps forward from a paused position advance exactly 10 frames.
    let path = media("h264_longgop.mp4");
    let mut lin = FfmpegVideoDecoder::open(&path, native()).unwrap();
    let pts: Vec<i64> = decode_all(&mut lin).iter().map(|f| f.pts_us).collect();
    let mut d = FfmpegVideoDecoder::open(&path, native()).unwrap();
    d.seek(pts[17]).unwrap();
    let mut cur = d.next_frame().unwrap().unwrap();
    for _ in 0..10 {
        cur = d.next_frame().unwrap().unwrap();
    }
    assert_eq!(cur.pts_us, pts[27]);
}

#[test]
fn display_output_formats() {
    let cases = [
        ("h264_longgop.mp4", PixelFormat::Nv12),
        ("hevc_main10_420.mp4", PixelFormat::P010),
        ("hevc_main422_10.mov", PixelFormat::P010),
        ("prores_hq.mov", PixelFormat::P010),
        ("prores_4444.mov", PixelFormat::Rgba8),
        ("dnxhr_hq.mxf", PixelFormat::Nv12),
        ("vp9_opus.webm", PixelFormat::Nv12),
    ];
    for (name, fmt) in cases {
        let mut d = open_video(&media(name), DecoderOptions::default()).unwrap();
        let f = d.next_frame().unwrap().unwrap();
        assert_eq!(f.format, fmt, "{name}");
        match fmt {
            PixelFormat::Nv12 => {
                assert_eq!(f.planes.len(), 2);
                assert_eq!(f.planes[0].data.len(), (f.width * f.height) as usize);
                assert_eq!(f.planes[1].data.len(), (f.width * f.height / 2) as usize);
            }
            PixelFormat::P010 => {
                assert_eq!(f.planes.len(), 2);
                assert_eq!(f.planes[0].stride, f.width as usize * 2);
            }
            PixelFormat::Rgba8 => {
                assert_eq!(f.planes.len(), 1);
                assert_eq!(f.planes[0].data.len(), (f.width * f.height * 4) as usize);
            }
            PixelFormat::Native(_) => unreachable!(),
        }
    }
}

/// Display repacking keeps luma exactly and takes chroma from the native planes (every other row/column for
/// 4:2:2 / 4:4:4 sources), shifted to the high bits for P010.
#[test]
fn display_repack_matches_native_planes() {
    for (name, depth) in
        [("h264_longgop.mp4", 8u32), ("dnxhr_hq.mxf", 8), ("prores_hq.mov", 10), ("hevc_main10_420.mp4", 10)]
    {
        let path = media(name);
        let mut nd = FfmpegVideoDecoder::open(&path, native()).unwrap();
        let mut dd =
            FfmpegVideoDecoder::open(&path, DecoderOptions { hwaccel: HwAccel::Off, ..Default::default() }).unwrap();
        let (n, d) = (nd.next_frame().unwrap().unwrap(), dd.next_frame().unwrap().unwrap());
        let (w, h) = (n.width as usize, n.height as usize);
        let sub_h = n.planes[1].height as usize == h; // 4:2:2 → chroma rows = luma rows
        let sub_w = n.planes[1].width as usize == w;
        let bytes = if depth > 8 { 2 } else { 1 };
        let rd = |p: &ove_media::Plane, x: usize, y: usize| -> u32 {
            let i = y * p.stride + x * bytes;
            if bytes == 1 { p.data[i] as u32 } else { u16::from_le_bytes([p.data[i], p.data[i + 1]]) as u32 }
        };
        let shift = if depth > 8 { 16 - depth } else { 0 };
        for y in (0..h).step_by(7) {
            for x in (0..w).step_by(5) {
                assert_eq!(rd(&d.planes[0], x, y), rd(&n.planes[0], x, y) << shift, "{name} Y({x},{y})");
            }
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        assert_eq!((d.planes[1].width as usize, d.planes[1].height as usize), (cw, ch));
        for cy in (0..ch).step_by(3) {
            for cx in (0..cw).step_by(3) {
                let (sx, sy) = (if sub_w { cx * 2 } else { cx }, if sub_h { cy * 2 } else { cy });
                let u = rd(&d.planes[1], cx * 2, cy);
                let v = rd(&d.planes[1], cx * 2 + 1, cy);
                assert_eq!(u, rd(&n.planes[1], sx, sy) << shift, "{name} U({cx},{cy})");
                assert_eq!(v, rd(&n.planes[2], sx, sy) << shift, "{name} V({cx},{cy})");
            }
        }
        assert!(!d.color.full_range, "{name}: video range");
    }
}

/// A "hardware" decoder for tests: decodes in software, reports a hardware path, and fails after `fail_after` frames.
struct FaultyHw {
    inner: FfmpegVideoDecoder,
    delivered: u64,
    fail_after: u64,
    fail_on_seek: bool,
}

impl VideoDecoder for FaultyHw {
    fn next_frame(&mut self) -> ove_media::Result<Option<VideoFrame>> {
        if self.delivered >= self.fail_after {
            return Err(MediaError::Hardware("injected: device removed".into()));
        }
        self.delivered += 1;
        Ok(self.inner.next_frame()?.map(|mut f| {
            f.decode_path = DecodePath::Hardware("fake".into());
            f
        }))
    }
    fn seek(&mut self, t: i64) -> ove_media::Result<()> {
        if self.fail_on_seek {
            return Err(MediaError::Hardware("injected: seek failed".into()));
        }
        self.inner.seek(t)
    }
    fn decode_path(&self) -> DecodePath {
        DecodePath::Hardware("fake".into())
    }
}

fn fallback_for(name: &str, fail_after: u64, fail_on_seek: bool) -> FallbackDecoder {
    let path = media(name);
    let p2 = path.clone();
    let primary =
        FaultyHw { inner: FfmpegVideoDecoder::open(&path, native()).unwrap(), delivered: 0, fail_after, fail_on_seek };
    FallbackDecoder::new(
        Box::new(primary),
        Box::new(move || Ok(Box::new(FfmpegVideoDecoder::open(&p2, native())?) as Box<dyn VideoDecoder>)),
    )
}

#[test]
fn hardware_failure_mid_stream_resumes_at_the_same_frame() {
    for name in ["h264_longgop.mp4", "hevc_main10_420.mp4", "vfr_phone.mp4"] {
        let reference = reference_framemd5(&media(name));
        let k = 37;
        let mut d = fallback_for(name, k, false);
        let mut frames = Vec::new();
        while let Some(f) = d.next_frame().unwrap() {
            frames.push(f);
        }
        assert_eq!(d.fallback_count(), 1, "{name}");
        assert!(d.fallback_reason().unwrap().contains("device removed"));
        assert_eq!(frames.len(), reference.len(), "{name}: no frame lost or repeated");
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(frame_md5(f), reference[i], "{name}: frame {i}");
            let expect = if (i as u64) < k { DecodePath::Hardware("fake".into()) } else { DecodePath::Software };
            assert_eq!(f.decode_path, expect, "{name}: path of frame {i}");
        }
        assert_eq!(d.decode_path(), DecodePath::Software);
    }
}

#[test]
fn hardware_failure_during_seek_falls_back() {
    let name = "h264_longgop.mp4";
    let reference = reference_framemd5(&media(name));
    let mut d = fallback_for(name, u64::MAX, true);
    d.seek(2_000_000).unwrap();
    let f = d.next_frame().unwrap().unwrap();
    assert_eq!(f.pts_us, 2_000_000);
    assert_eq!(frame_md5(&f), reference[60]);
    assert_eq!(d.fallback_count(), 1);
}

#[test]
fn injected_failure_on_real_decoder_option_is_inert_without_hardware() {
    // On machines without a hardware decoder the option must not fire (it only applies to the hardware path).
    let opts = DecoderOptions { inject_hw_failure_after: Some(3), output: OutputFormat::Native, ..Default::default() };
    let mut d = open_video(&media("h264_longgop.mp4"), opts).unwrap();
    let mut n = 0;
    while d.next_frame().unwrap().is_some() {
        n += 1;
    }
    let hw = !matches!(d.decode_path(), DecodePath::Software) || d.fallback_count() > 0;
    if !hw {
        assert_eq!(n, 120);
    } else {
        // On a machine with hardware decode: the injected failure switched to software without losing frames.
        assert_eq!(n, 120);
        assert_eq!(d.fallback_count(), 1);
    }
}

#[test]
fn truncated_file_ends_cleanly() {
    let mut d = FfmpegVideoDecoder::open(&media("truncated_faststart.mp4"), native()).unwrap();
    let mut n = 0;
    loop {
        match d.next_frame() {
            Ok(Some(_)) => n += 1,
            Ok(None) => break,
            Err(e) => panic!("truncated file should end, not fail: {e}"),
        }
    }
    assert!(n > 10 && n < 120, "decoded {n} frames from the truncated part");
    let e = open_video(&media("truncated_nomoov.mp4"), DecoderOptions::default()).err().unwrap();
    assert_eq!(e.code(), "media.corrupt");
}

#[test]
fn audio_only_file_has_no_video_stream() {
    let e = open_video(&media("audio_aac.m4a"), DecoderOptions::default()).err().unwrap();
    assert!(matches!(e, MediaError::NoStream("video")), "{e:?}");
}

#[test]
fn deleted_file_goes_offline() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("clip.mxf");
    std::fs::copy(media("dnxhr_hq.mxf"), &copy).unwrap();
    let mut d = FfmpegVideoDecoder::open(&copy, native()).unwrap();
    for _ in 0..3 {
        d.next_frame().unwrap().unwrap();
    }
    std::fs::remove_file(&copy).unwrap();
    let err = loop {
        match d.next_frame() {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("reached the end without noticing the file was removed"),
            Err(e) => break e,
        }
    };
    assert_eq!(err.code(), "media.offline", "{err}");
}
