//! Golden-image tests for the viewer renderer, run headless on any adapter (a software adapter
//! such as lavapipe or WARP is fine). Skipped with a message when no adapter exists.
//!
//! Regenerate goldens with `OVE_UPDATE_GOLDEN=1 cargo test -p ove-gpu --test golden`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::PathBuf;

use ove_gpu::cpu::{frame_to_rgba, range_params};
use ove_gpu::{GpuContext, GpuOptions, PixelFormat, VideoFrame, ViewerRenderer, YuvMatrix};

const BARS: [[u8; 3]; 8] = [
    [235, 235, 235],
    [235, 235, 16],
    [16, 235, 235],
    [16, 235, 16],
    [235, 16, 235],
    [235, 16, 16],
    [16, 16, 235],
    [16, 16, 16],
];

fn renderer() -> Option<ViewerRenderer> {
    match GpuContext::new(GpuOptions { allow_software: true }) {
        Ok(ctx) => {
            eprintln!("adapter: {}", ctx.summary());
            Some(ViewerRenderer::new(ctx))
        }
        Err(e) => {
            eprintln!("SKIPPED: no GPU adapter available ({e})");
            None
        }
    }
}

/// Limited-range BT.709 Y'CbCr code values (8-bit) for an RGB colour in 16..235 studio range.
fn rgb_to_ycbcr(rgb: [u8; 3]) -> (f32, f32, f32) {
    let (kr, kb) = YuvMatrix::Bt709.coefficients();
    let n = |v: u8| (f32::from(v) - 16.0) / 219.0;
    let (r, g, b) = (n(rgb[0]), n(rgb[1]), n(rgb[2]));
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    (16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr)
}

/// 160×90 colour bars with a luma ramp in the bottom quarter.
fn bars(format: PixelFormat) -> VideoFrame {
    let (w, h) = (160u32, 90u32);
    let sample = |x: u32, y: u32| -> (f32, f32, f32) {
        if y >= 68 {
            (16.0 + 219.0 * x as f32 / (w - 1) as f32, 128.0, 128.0)
        } else {
            rgb_to_ycbcr(BARS[(x / 20) as usize])
        }
    };
    match format {
        PixelFormat::Nv12 => {
            let mut yp = vec![0u8; (w * h) as usize];
            let mut uv = vec![0u8; (w * h / 2) as usize];
            for y in 0..h {
                for x in 0..w {
                    yp[(y * w + x) as usize] = sample(x, y).0.round() as u8;
                }
            }
            for cy in 0..h / 2 {
                for cx in 0..w / 2 {
                    let (_, u, v) = sample(cx * 2, cy * 2);
                    uv[(cy * w + cx * 2) as usize] = u.round() as u8;
                    uv[(cy * w + cx * 2 + 1) as usize] = v.round() as u8;
                }
            }
            VideoFrame::nv12(w, h, yp, uv, 0)
        }
        PixelFormat::P010 => {
            let word = |v: f32| (((v * 4.0).round() as u16) << 6).to_le_bytes();
            let mut yp = Vec::with_capacity((w * h * 2) as usize);
            for y in 0..h {
                for x in 0..w {
                    yp.extend_from_slice(&word(sample(x, y).0));
                }
            }
            let mut uv = Vec::with_capacity((w * h) as usize * 2 / 2);
            for cy in 0..h / 2 {
                for cx in 0..w / 2 {
                    let (_, u, v) = sample(cx * 2, cy * 2);
                    uv.extend_from_slice(&word(u));
                    uv.extend_from_slice(&word(v));
                }
            }
            VideoFrame {
                width: w,
                height: h,
                format: PixelFormat::P010,
                planes: vec![yp.into(), uv.into()],
                strides: vec![w * 2, w * 2],
                pts: 0,
                matrix: YuvMatrix::Bt709,
                full_range: false,
            }
        }
        PixelFormat::Rgba8 => {
            let nv12 = bars(PixelFormat::Nv12);
            VideoFrame::rgba(w, h, frame_to_rgba(&nv12), 0)
        }
    }
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn compare_golden(name: &str, width: u32, height: u32, pixels: &[u8]) {
    let path = golden_path(name);
    if std::env::var_os("OVE_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(pixels).unwrap();
        eprintln!("updated {}", path.display());
        return;
    }
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("missing golden {} ({e}); run with OVE_UPDATE_GOLDEN=1", path.display()));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().unwrap();
    let mut golden = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut golden).unwrap();
    assert_eq!((info.width, info.height), (width, height), "golden size");
    let mut worst = 0u8;
    let mut over = 0usize;
    for (a, b) in pixels.iter().zip(&golden) {
        let d = a.abs_diff(*b);
        worst = worst.max(d);
        if d > 2 {
            over += 1;
        }
    }
    // Tolerance absorbs rounding differences between GPU vendors/drivers.
    assert!(worst <= 6 && over * 1000 <= pixels.len(), "{name}: max channel diff {worst}, {over} channels over 2");
}

fn px(img: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

fn check_format(format: PixelFormat, golden: &str) {
    let Some(mut r) = renderer() else { return };
    let frame = bars(format);
    r.upload(&frame).unwrap();
    // 16:9 frame into a 4:3 target → letterboxed at 2× scale: image 320×180 at y = 30.
    let (img, info) = r.render_to_rgba(320, 240).unwrap();
    assert_eq!((info.image.x, info.image.y, info.image.width, info.image.height), (0, 30, 320, 180));

    // Bars must match the CPU reference conversion at bar centres, and bars stay black.
    let reference = frame_to_rgba(&frame);
    for bar in 0..8u32 {
        let (fx, fy) = (bar * 20 + 10, 30);
        let want = px(&reference, 160, fx, fy);
        let got = px(&img, 320, fx * 2 + 1, 30 + fy * 2 + 1);
        for c in 0..3 {
            assert!(got[c].abs_diff(want[c]) <= 3, "{format:?} bar {bar}: got {got:?} want {want:?}");
        }
    }
    assert_eq!(px(&img, 320, 160, 5), [0, 0, 0, 255], "letterbox bar is background");
    assert_eq!(px(&img, 320, 160, 235), [0, 0, 0, 255], "letterbox bar is background");
    compare_golden(golden, 320, 240, &img);
}

#[test]
fn nv12_bars_golden() {
    check_format(PixelFormat::Nv12, "bars_nv12_320x240.png");
}

#[test]
fn p010_bars_golden() {
    // Same picture at 10 bits renders to the same 8-bit output.
    check_format(PixelFormat::P010, "bars_nv12_320x240.png");
}

#[test]
fn rgba_bars_golden() {
    check_format(PixelFormat::Rgba8, "bars_rgba_320x240.png");
}

#[test]
fn range_params_are_consistent() {
    assert_eq!(range_params(10, false), [64.0, 876.0, 512.0, 896.0]);
}

#[test]
fn recovers_from_device_loss() {
    let Some(mut r) = renderer() else { return };
    r.upload(&bars(PixelFormat::Nv12)).unwrap();
    let (before, _) = r.render_to_rgba(320, 180).unwrap();
    let generation = r.context().generation;

    r.context().simulate_device_loss();
    assert!(r.context().is_lost());
    let started = std::time::Instant::now();
    assert!(r.recover_if_lost(None).unwrap(), "recovery happened");
    let (after, _) = r.render_to_rgba(320, 180).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(r.context().generation, generation + 1);
    assert_eq!(before, after, "current frame re-uploaded and shown identically");
    assert!(elapsed.as_secs_f64() < 2.0, "recovered in {elapsed:?} (spec: within 2 s)");
    eprintln!("device-loss recovery took {elapsed:?}");
    // No-op when healthy.
    assert!(!r.recover_if_lost(None).unwrap());
}

#[test]
fn rejects_invalid_frames_without_panicking() {
    let Some(mut r) = renderer() else { return };
    let mut frame = bars(PixelFormat::Nv12);
    frame.planes[1] = vec![0u8; 10].into();
    assert!(r.upload(&frame).is_err());
    // Renders background only when nothing valid was uploaded.
    let (img, info) = r.render_to_rgba(8, 8).unwrap();
    assert_eq!(info.image.width, 0);
    assert!(img.chunks(4).all(|p| p == [0, 0, 0, 255]));
}

#[test]
fn upload_stats_count_copies() {
    let Some(mut r) = renderer() else { return };
    r.upload(&bars(PixelFormat::Nv12)).unwrap();
    r.upload(&bars(PixelFormat::Nv12)).unwrap();
    use std::sync::atomic::Ordering;
    assert_eq!(r.stats.frames.load(Ordering::Relaxed), 2);
    assert_eq!(r.stats.copies_last_frame.load(Ordering::Relaxed), 1);
    assert_eq!(r.stats.bytes.load(Ordering::Relaxed), 2 * (160 * 90 + 160 * 45));
}
