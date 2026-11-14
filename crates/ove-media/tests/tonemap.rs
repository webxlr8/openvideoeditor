//! HDR → SDR: PQ and HLG sources come out of RGBA/Display decoding tone-mapped (grey card, skin tone and reference
//! white land where SDR expects them), match the f64 reference pipeline, and the tables are fast.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod common;

use common::media;
use ove_media::tonemap::{YuvParams, YuvSource, reference_rgb, tone_map_yuv};
use ove_media::{
    DecoderOptions, FfmpegVideoDecoder, HwAccel, OutputFormat, PixelFormat, Transfer, VideoDecoder, VideoFrame,
};

fn first_frame(name: &str, output: OutputFormat) -> VideoFrame {
    let opts = DecoderOptions { hwaccel: HwAccel::Off, output, inject_hw_failure_after: None };
    FfmpegVideoDecoder::open(&media(name), opts).unwrap().next_frame().unwrap().unwrap()
}

/// 10-bit code value of a native yuv420p10le plane.
fn code(f: &VideoFrame, plane: usize, x: usize, y: usize) -> f64 {
    let p = &f.planes[plane];
    let o = y * p.stride + x * 2;
    f64::from(u16::from_le_bytes([p.data[o], p.data[o + 1]]))
}

/// Limited-range BT.2020 Y'CbCr codes (10-bit) → non-linear R'G'B' 0–1.
fn rgb_of(y: f64, cb: f64, cr: f64) -> [f64; 3] {
    let (kr, kb) = (0.2627, 0.0593);
    let kg = 1.0 - kr - kb;
    let (y, cb, cr) = ((y - 64.0) / 876.0, (cb - 512.0) / 896.0, (cr - 512.0) / 896.0);
    [
        y + 2.0 * (1.0 - kr) * cr,
        y - 2.0 * kb * (1.0 - kb) / kg * cb - 2.0 * kr * (1.0 - kr) / kg * cr,
        y + 2.0 * (1.0 - kb) * cb,
    ]
    .map(|c| c.clamp(0.0, 1.0))
}

/// Mean RGB of a `size`×`size` block of an RGBA8 frame.
fn block_mean(f: &VideoFrame, cx: usize, cy: usize, size: usize) -> [f64; 3] {
    let p = &f.planes[0];
    let mut sum = [0f64; 3];
    for y in cy - size / 2..cy + size / 2 {
        for x in cx - size / 2..cx + size / 2 {
            for (c, s) in sum.iter_mut().enumerate() {
                *s += f64::from(p.data[y * p.stride + x * 4 + c]);
            }
        }
    }
    sum.map(|s| s / (size * size) as f64)
}

#[test]
fn hdr_display_output_is_tone_mapped_and_display_hdr_keeps_ten_bits() {
    if !common::ready() {
        return;
    }
    let f = first_frame("hevc_hdr_vfr.mov", OutputFormat::Display);
    assert_eq!(f.format, PixelFormat::Rgba8, "HDR is tone-mapped for viewers that do not know HDR");
    assert!(f.color.hdr);
    assert_eq!(f.color.transfer, Transfer::Sdr);
    assert_eq!((f.width, f.height), (1280, 720));
    let f = first_frame("hevc_hdr_vfr.mov", OutputFormat::DisplayHdr);
    assert_eq!(f.format, PixelFormat::P010);
    assert_eq!(f.color.transfer, Transfer::Pq);
    assert!(f.color.bt2020_primaries);
    let f = first_frame("hevc_hlg_patches.mov", OutputFormat::DisplayHdr);
    assert_eq!(f.color.transfer, Transfer::Hlg);
    // SDR sources are untouched.
    let f = first_frame("hevc_main10_420.mp4", OutputFormat::Display);
    assert_eq!((f.format, f.color.hdr, f.color.transfer), (PixelFormat::P010, false, Transfer::Sdr));
}

#[test]
fn grey_card_skin_and_reference_white_land_in_the_sdr_range() {
    if !common::ready() {
        return;
    }
    for (name, transfer) in [("hevc_hlg_patches.mov", Transfer::Hlg), ("hevc_pq_patches.mov", Transfer::Pq)] {
        let native = first_frame(name, OutputFormat::Native);
        let full = first_frame(name, OutputFormat::Rgba { max_width: 0, max_height: 0 });
        let half = first_frame(name, OutputFormat::Rgba { max_width: 0, max_height: 360 });
        assert_eq!((half.width, half.height), (640, 360));
        let mut got = Vec::new();
        for band in 0..3 {
            let (x, y) = (1280 * (2 * band + 1) / 6, 360);
            let want = reference_rgb(
                transfer,
                rgb_of(code(&native, 0, x, y), code(&native, 1, x / 2, y / 2), code(&native, 2, x / 2, y / 2)),
                true,
            )
            .map(|v| v * 255.0);
            let out = block_mean(&full, x, y, 16);
            let small = block_mean(&half, x / 2, y / 2, 8);
            for c in 0..3 {
                assert!((out[c] - want[c]).abs() <= 2.0, "{name} band {band}: {out:?} vs reference {want:?}");
                assert!((small[c] - want[c]).abs() <= 2.0, "{name} band {band} scaled: {small:?} vs {want:?}");
            }
            got.push(out);
        }
        let [grey, skin, white] = [got[0], got[1], got[2]];
        eprintln!("{name}: grey {grey:.0?}, skin {skin:.0?}, white {white:.0?}");
        // 18 % grey through the BT.709 OETF is code 104; neutral stays neutral.
        assert!(grey.iter().all(|v| (96.0..=112.0).contains(v)), "{name}: grey card {grey:?}");
        assert!(grey[0].max(grey[2]) - grey[0].min(grey[2]) <= 3.0, "{name}: grey is neutral {grey:?}");
        // Reference white sits below SDR white, leaving room for highlights; never washed out to 255.
        assert!(white.iter().all(|v| (215.0..=240.0).contains(v)), "{name}: reference white {white:?}");
        // Skin: warm (R > G > B) and in the usual mid-to-light SDR range.
        assert!(skin[0] > skin[1] && skin[1] > skin[2], "{name}: skin hue {skin:?}");
        assert!((150.0..=230.0).contains(&skin[0]) && (70.0..=170.0).contains(&skin[2]), "{name}: skin {skin:?}");
    }
}

#[test]
fn pq_bars_match_the_reference_pipeline() {
    if !common::ready() {
        return;
    }
    let native = first_frame("hevc_hdr_vfr.mov", OutputFormat::Native);
    let out = first_frame("hevc_hdr_vfr.mov", OutputFormat::Rgba { max_width: 0, max_height: 0 });
    assert_eq!(native.format, PixelFormat::Native("yuv420p10le".into()));
    let (mut checked, mut worst) = (0, 0f64);
    for y in (8..712).step_by(6) {
        for x in (8..1272).step_by(6) {
            // Only where chroma is locally flat, so chroma interpolation cannot matter.
            let (cx, cy) = (x / 2, y / 2);
            let flat = (1..3).all(|p| {
                let c = code(&native, p, cx, cy);
                (cy - 1..=cy + 1).all(|yy| (cx - 1..=cx + 1).all(|xx| (code(&native, p, xx, yy) - c).abs() <= 1.0))
            });
            if !flat {
                continue;
            }
            let want = reference_rgb(
                Transfer::Pq,
                rgb_of(code(&native, 0, x, y), code(&native, 1, cx, cy), code(&native, 2, cx, cy)),
                true,
            );
            let p = &out.planes[0];
            for (c, w) in want.iter().enumerate() {
                let got = f64::from(p.data[y * p.stride + x * 4 + c]);
                worst = worst.max((got - w * 255.0).abs());
            }
            checked += 1;
        }
    }
    eprintln!("PQ bars: {checked} pixels checked, worst channel difference {worst:.2}");
    assert!(checked > 5000, "{checked}");
    assert!(worst <= 2.0, "worst {worst}");
}

#[test]
fn tone_map_speed_1080p() {
    // A 1920×1080 4:2:0 10-bit ramp, no media needed.
    let (w, h) = (1920usize, 1080usize);
    let y: Vec<u8> = (0..w * h).flat_map(|i| ((64 + (i % w) * 876 / w) as u16).to_le_bytes()).collect();
    let c: Vec<u8> = (0..w / 2 * h / 2).flat_map(|i| ((400 + (i % 200)) as u16).to_le_bytes()).collect();
    let src = YuvSource {
        width: w,
        height: h,
        depth: 10,
        log2_chroma_w: 1,
        log2_chroma_h: 1,
        y: (&y, w * 2),
        cb: (&c, w),
        cr: Some((&c, w)),
        msb_aligned: false,
    };
    let params = YuvParams { kr: 0.2627, kb: 0.0593, full_range: false };
    let mut out = vec![0u8; w * h * 4];
    tone_map_yuv(&src, params, Transfer::Hlg, true, &mut out); // warm-up (tables, thread pool)
    let n = 20;
    let t = std::time::Instant::now();
    for _ in 0..n {
        tone_map_yuv(&src, params, Transfer::Pq, true, &mut out);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
    eprintln!(
        "PERF tone-map 1920x1080 10-bit 4:2:0 → RGBA8: {ms:.2} ms/frame ({} threads)",
        rayon::current_num_threads()
    );
    assert!(ms < 250.0, "{ms} ms per frame");
}
