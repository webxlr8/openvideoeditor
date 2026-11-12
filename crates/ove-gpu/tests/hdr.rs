//! HDR tone mapping in the viewer agrees with the compositor: the WGSL pass, the CPU reference
//! (`ove_gpu::cpu`) and `ove_media`'s table-driven RGBA output give the same SDR picture for PQ and
//! HLG footage. GPU checks are skipped with a message when no adapter exists.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

#[path = "../../../tests/media/fixtures.rs"]
mod fixtures;

use fixtures::{media, media_ready};
use ove_gpu::cpu::{frame_to_rgba_with_transfer, tone_map_rgb};
use ove_gpu::{GpuContext, GpuOptions, PixelFormat, Transfer, VideoFrame, ViewerRenderer, YuvMatrix};
use ove_media::{DecoderOptions, FfmpegVideoDecoder, HwAccel, OutputFormat, VideoDecoder};

fn renderer() -> Option<ViewerRenderer> {
    match GpuContext::new(GpuOptions { allow_software: true }) {
        Ok(ctx) => {
            eprintln!("adapter: {}", ctx.summary());
            Some(ViewerRenderer::new(ctx))
        }
        Err(e) => {
            eprintln!("SKIPPED (GPU part): no GPU adapter available ({e})");
            None
        }
    }
}

fn decode(name: &str, output: OutputFormat) -> ove_media::VideoFrame {
    let opts = DecoderOptions { hwaccel: HwAccel::Off, output, inject_hw_failure_after: None };
    FfmpegVideoDecoder::open(&media(name), opts).unwrap().next_frame().unwrap().unwrap()
}

/// The single-file viewer's path: P010 with its transfer, as the desktop hands it to the GPU.
fn viewer_frame(f: ove_media::VideoFrame) -> (VideoFrame, Transfer) {
    assert_eq!(f.format, ove_media::PixelFormat::P010);
    let transfer = match f.color.transfer {
        ove_media::Transfer::Pq => Transfer::Pq,
        ove_media::Transfer::Hlg => Transfer::Hlg,
        ove_media::Transfer::Sdr => Transfer::Sdr,
    };
    assert_eq!(f.color.matrix, ove_media::ColorMatrix::Bt2020);
    let frame = VideoFrame {
        width: f.width,
        height: f.height,
        format: PixelFormat::P010,
        strides: f.planes.iter().map(|p| p.stride as u32).collect(),
        planes: f.planes.into_iter().map(|p| p.data.into()).collect(),
        pts: f.pts_us,
        matrix: YuvMatrix::Bt2020,
        full_range: f.color.full_range,
    };
    (frame, transfer)
}

/// Pixels whose 3×3 chroma neighbourhood is flat (±1 code), so chroma siting and interpolation
/// differences between the paths cannot matter.
fn flat_chroma_pixels(f: &VideoFrame) -> Vec<(usize, usize)> {
    let (cw, ch) = (f.width.div_ceil(2) as usize, f.height.div_ceil(2) as usize);
    let uv = &f.planes[1];
    let s = f.strides[1] as usize;
    let at = |x: usize, y: usize, c: usize| {
        u16::from_le_bytes([uv[y * s + x * 4 + c * 2], uv[y * s + x * 4 + c * 2 + 1]]) >> 6
    };
    let mut out = Vec::new();
    for cy in (1..ch - 1).step_by(3) {
        for cx in (1..cw - 1).step_by(3) {
            let flat = (0..2).all(|c| {
                let v = at(cx, cy, c);
                (cy - 1..=cy + 1).all(|y| (cx - 1..=cx + 1).all(|x| at(x, y, c).abs_diff(v) <= 1))
            });
            if flat {
                out.push((cx * 2, cy * 2));
            }
        }
    }
    out
}

fn px(img: &[u8], w: u32, x: usize, y: usize) -> [u8; 3] {
    let i = (y * w as usize + x) * 4;
    [img[i], img[i + 1], img[i + 2]]
}

#[test]
fn cpu_reference_matches_ove_media_math() {
    let mut worst = 0f64;
    for t in [Transfer::Pq, Transfer::Hlg] {
        let mt = if t == Transfer::Pq { ove_media::Transfer::Pq } else { ove_media::Transfer::Hlg };
        for r in (0..=1023).step_by(33) {
            for g in (0..=1023).step_by(41) {
                for b in (0..=1023).step_by(47) {
                    let rgb = [r, g, b].map(|v| v as f64 / 1023.0);
                    let want = ove_media::tonemap::reference_rgb(mt, rgb, true);
                    let got = tone_map_rgb(t, rgb.map(|v| v as f32));
                    for c in 0..3 {
                        worst = worst.max((f64::from(got[c]) - want[c]).abs() * 255.0);
                    }
                }
            }
        }
    }
    assert!(worst < 0.6, "f32 CPU reference vs f64 ove-media reference: {worst} codes");
}

#[test]
fn gpu_cpu_and_compositor_agree_on_hdr_footage() {
    if !media_ready() {
        return;
    }
    let mut gpu = renderer();
    for name in ["hevc_hdr_vfr.mov", "hevc_pq_patches.mov", "hevc_hlg_patches.mov"] {
        let (frame, transfer) = viewer_frame(decode(name, OutputFormat::DisplayHdr));
        assert_ne!(transfer, Transfer::Sdr, "{name}");
        let (w, h) = (frame.width, frame.height);
        let reference = frame_to_rgba_with_transfer(&frame, transfer);
        let compositor = decode(name, OutputFormat::Rgba { max_width: 0, max_height: 0 });
        let flat = flat_chroma_pixels(&frame);
        assert!(flat.len() > 2000, "{name}: {} flat pixels", flat.len());

        // Compositor (ove-media tables, interpolated chroma) vs the CPU reference.
        let mut worst = 0u8;
        for &(x, y) in &flat {
            let a = px(&reference, w, x, y);
            let b = px(&compositor.planes[0].data, w, x, y);
            worst = worst.max((0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap());
        }
        eprintln!("{name}: compositor vs CPU reference, worst {worst} over {} pixels", flat.len());
        assert!(worst <= 3, "{name}: compositor differs from the viewer reference by {worst}");

        // GPU (WGSL) vs the CPU reference, every pixel at 1:1.
        if let Some(r) = gpu.as_mut() {
            r.upload_with_transfer(&frame, transfer).unwrap();
            assert_eq!(r.current_transfer(), transfer);
            let (img, info) = r.render_to_rgba(w, h).unwrap();
            assert_eq!((info.image.width, info.image.height), (w, h));
            let (mut worst, mut over) = (0u8, 0usize);
            for (i, (a, b)) in img.iter().zip(&reference).enumerate() {
                if i % 4 == 3 {
                    continue;
                }
                let d = a.abs_diff(*b);
                worst = worst.max(d);
                over += usize::from(d > 2);
            }
            eprintln!("{name}: GPU vs CPU reference, worst {worst}, {over} channels over 2");
            assert!(worst <= 4 && over * 1000 <= img.len(), "{name}: GPU differs: worst {worst}, {over} over 2");
            if name == "hevc_hlg_patches.mov" || name == "hevc_pq_patches.mov" {
                let grey = px(&img, w, w as usize / 6, h as usize / 2);
                assert!(grey.iter().all(|v| (96..=112).contains(v)), "{name}: GPU grey card {grey:?}");
            }
        }
    }
}

#[test]
fn sdr_upload_ignores_tone_mapping() {
    let Some(mut r) = renderer() else { return };
    // An RGBA frame uploaded "as HLG" is shown untouched.
    let frame = VideoFrame::rgba(2, 1, vec![10, 20, 30, 255, 200, 100, 50, 255], 0);
    r.upload_with_transfer(&frame, Transfer::Hlg).unwrap();
    let (img, _) = r.render_to_rgba(2, 1).unwrap();
    assert_eq!(img, vec![10, 20, 30, 255, 200, 100, 50, 255]);
    assert_eq!(r.current_transfer(), Transfer::Sdr);
}
