//! Proxies (add-background-tasks-and-media-derivatives, task 5.1 and 5.2): which files need one, the encode (1080p
//! H.264, a keyframe every 12 frames, SSIM ≥ 0.97 against the original at 1080p, HDR tone-mapped), and that playback
//! reads proxies while export never does. Needs the fixture ffmpeg CLI to make the media (`OVE_FIXTURE_FFMPEG`):
//! without it the tests fail, unless `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` makes them print "skipped".
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
use ove_media::proxy::{self, Reason};
use ove_media::{DecoderOptions, FfmpegVideoDecoder, HwAccel, OutputFormat, VideoDecoder};
use ove_render::{ExportSettings, Renderer, export};
use serde_json::json;

fn tools() -> bool {
    fixtures::require_tool(&fixtures::fixture_ffmpeg(), "it makes the proxy test media; set OVE_FIXTURE_FFMPEG")
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let o = Command::new(fixtures::fixture_ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "ffmpeg failed for {name}: {}", String::from_utf8_lossy(&o.stderr));
    out
}

/// 2 s of a moving test pattern at 24 fps, `size`, with the given encoder arguments.
fn pattern(dir: &Path, name: &str, size: &str, codec: &[&str]) -> PathBuf {
    let src = format!("testsrc2=s={size}:r=24:d=2");
    let mut args = vec!["-f", "lavfi", "-i", src.as_str()];
    args.extend_from_slice(codec);
    make(dir, name, &args)
}

fn uhd(dir: &Path) -> PathBuf {
    pattern(dir, "uhd.mp4", "3840x2160", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "48", "-crf", "16"])
}

fn deep(dir: &Path) -> PathBuf {
    pattern(
        dir,
        "deep.mp4",
        "1280x720",
        &["-c:v", "libx265", "-pix_fmt", "yuv420p10le", "-x265-params", "keyint=48:log-level=error", "-crf", "16"],
    )
}

/// HDR10: 10-bit HEVC tagged BT.2020 / PQ.
fn hdr(dir: &Path) -> PathBuf {
    pattern(
        dir,
        "hdr.mp4",
        "1920x1080",
        &[
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p10le",
            "-x265-params",
            "keyint=48:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:hdr10=1:log-level=error",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-crf",
            "16",
        ],
    )
}

fn intra(dir: &Path) -> PathBuf {
    pattern(dir, "intra.mov", "1280x720", &["-c:v", "prores_ks", "-profile:v", "1", "-pix_fmt", "yuv422p10le"])
}

fn small(dir: &Path) -> PathBuf {
    pattern(dir, "small.mp4", "640x360", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "48"])
}

#[test]
fn eligibility_follows_size_depth_and_decode_speed() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (uhd(dir.path()), Some(Reason::Resolution)),
        (deep(dir.path()), Some(Reason::DeepLongGop)),
        (hdr(dir.path()), Some(Reason::DeepLongGop)),
        // 10-bit but intra-only, and small 8-bit long-GOP: both decode faster than real time here.
        (intra(dir.path()), None),
        (small(dir.path()), None),
    ];
    for (path, want) in cases {
        let info = ove_media::probe(&path).unwrap();
        let c = proxy::check(&path, &info).unwrap();
        eprintln!("{}: {:?} (decode speed {:?}× real time)", path.display(), c.reason, c.decode_speed);
        assert_eq!(c.reason, want, "{}", path.display());
        if want.is_none() {
            assert!(c.decode_speed.unwrap() >= proxy::REALTIME_MARGIN);
        }
    }
}

/// RGBA frames of a file decoded to fit 1920 × 1080 (HDR tone-mapped to SDR), at most `n`.
fn frames_1080(path: &Path, n: usize) -> Vec<(u32, u32, Vec<u8>)> {
    let opts = DecoderOptions {
        hwaccel: HwAccel::Off,
        output: OutputFormat::Rgba { max_width: 1920, max_height: 1080 },
        ..Default::default()
    };
    let mut d = FfmpegVideoDecoder::open(path, opts).unwrap();
    let mut v = Vec::new();
    while v.len() < n {
        let Some(f) = d.next_frame().unwrap() else { break };
        let p = &f.planes[0];
        let row = f.width as usize * 4;
        let data: Vec<u8> = p.data.chunks(p.stride).take(f.height as usize).flat_map(|r| r[..row].to_vec()).collect();
        v.push((f.width, f.height, data));
    }
    v
}

/// Luma of an RGBA image cropped to `w` × `h`.
fn luma(img: &(u32, u32, Vec<u8>), w: u32, h: u32) -> Vec<f64> {
    let stride = img.0 as usize * 4;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = &img.2[y * stride + x * 4..];
            out.push(0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64);
        }
    }
    out
}

/// Mean SSIM of two luma images over 8 × 8 windows, every 4 pixels.
fn ssim(a: &[f64], b: &[f64], w: usize, h: usize) -> f64 {
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let (mut sum, mut n) = (0.0, 0u64);
    for y in (0..h.saturating_sub(8)).step_by(4) {
        for x in (0..w.saturating_sub(8)).step_by(4) {
            let (mut ma, mut mb) = (0.0, 0.0);
            for j in 0..8 {
                for i in 0..8 {
                    ma += a[(y + j) * w + x + i];
                    mb += b[(y + j) * w + x + i];
                }
            }
            ma /= 64.0;
            mb /= 64.0;
            let (mut va, mut vb, mut cov) = (0.0, 0.0, 0.0);
            for j in 0..8 {
                for i in 0..8 {
                    let (da, db) = (a[(y + j) * w + x + i] - ma, b[(y + j) * w + x + i] - mb);
                    va += da * da;
                    vb += db * db;
                    cov += da * db;
                }
            }
            va /= 63.0;
            vb /= 63.0;
            cov /= 63.0;
            sum += ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1;
        }
    }
    sum / n.max(1) as f64
}

/// The proxy is 1080p H.264 with a keyframe every 12 frames, and matches the original (SSIM ≥ 0.97 at 1080p),
/// for a 4K source, a 10-bit long-GOP source and an HDR (PQ) source.
#[test]
fn proxies_are_1080p_short_gop_and_match_the_original() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for (name, src) in [("4K", uhd(dir.path())), ("10-bit", deep(dir.path())), ("HDR", hdr(dir.path()))] {
        let out = dir.path().join(format!("proxies/{name}_1080.mp4"));
        let mut calls = 0;
        let r = proxy_make(&src, &out, &mut calls);
        let info = ove_media::probe(&out).unwrap();
        let v = info.video.as_ref().unwrap();
        assert_eq!(v.codec, "h264", "{name}");
        assert_eq!(v.pixel_format.as_ref().unwrap().bit_depth, 8, "{name}");
        let src_info = ove_media::probe(&src).unwrap();
        let sv = src_info.video.as_ref().unwrap();
        assert_eq!((v.width, v.height), proxy::frame_size(sv.width, sv.height), "{name}");
        assert!(v.height <= 1080 && v.width <= 1920);
        assert!(info.audio.is_none(), "{name}: proxies carry no sound");
        assert!(calls > 0, "progress was reported");

        // Keyframes every 12 frames: 48 frames at 24 fps give keyframes at 0, 0.5, 1 and 1.5 s.
        let mut d = FfmpegVideoDecoder::open(&out, Default::default()).unwrap();
        let keys: std::collections::BTreeSet<i64> =
            (0..48).filter_map(|i| d.keyframe_before(i * 1_000_000 / 24)).collect();
        assert_eq!(keys.len(), 4, "{name}: keyframes {keys:?}");

        let a = frames_1080(&src, 48);
        let b = frames_1080(&out, 48);
        assert_eq!(a.len(), b.len(), "{name}: frame count");
        let (w, h) = (b[0].0.min(a[0].0), b[0].1.min(a[0].1));
        let mut worst = 1.0f64;
        let mut total = 0.0;
        for (x, y) in a.iter().zip(&b).step_by(4) {
            let s = ssim(&luma(x, w, h), &luma(y, w, h), w as usize, h as usize);
            worst = worst.min(s);
            total += s;
        }
        let mean = total / a.iter().step_by(4).count() as f64;
        eprintln!(
            "{name}: proxy {}x{} by {:?} in {:.2} s for {} frames; SSIM mean {mean:.4}, worst {worst:.4}",
            r.width, r.height, r.encoder, r.seconds_taken, r.frames
        );
        eprintln!("{name}: proxy file {} KB", std::fs::metadata(&out).unwrap().len() / 1024);
        assert!(worst >= 0.97, "{name}: SSIM {worst:.4}");
    }
}

fn proxy_make(src: &Path, out: &Path, calls: &mut u32) -> ove_render::proxy::ProxyResult {
    ove_render::proxy::make(src, out, |f| {
        assert!((0.0..=1.0).contains(&f));
        *calls += 1;
        true
    })
    .unwrap()
}

#[test]
fn a_cancelled_proxy_leaves_nothing() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = deep(dir.path());
    let out = dir.path().join("proxies/c_1080.mp4");
    let r = ove_render::proxy::make(&src, &out, |f| f < 0.3);
    assert!(matches!(r, Err(ove_render::ExportError::Cancelled)), "{r:?}");
    let left: Vec<_> = std::fs::read_dir(out.parent().unwrap()).unwrap().flatten().collect();
    assert!(left.is_empty(), "{left:?}");
}

/// A project with one clip whose content id is `cid`, in a bundle folder.
fn clip_doc(clip: &Path, cid: &str, use_proxies: bool) -> Document {
    let mut ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 0, "type": "video"}) },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "video", "fileName": "clip.mp4", "duration": 1, "hasAudio": false}}),
            entry: Some(json!({"mediaId": "m", "nativePath": clip.display().to_string(), "ownedByBundle": false,
                               "contentId": cid})),
        },
        Op::AddItem {
            item: json!({"id": "v", "trackId": "main", "type": "video", "startTime": 0, "endTime": 1, "mediaItemId": "m"}),
        },
    ];
    if use_proxies {
        let mut set = serde_json::Map::new();
        set.insert("useProxies".into(), json!(true));
        ops.push(Op::UpdateProject { set, unset: vec![] });
    }
    apply_batch(&Document::new("p", "Proxies", 1), &ops).unwrap().doc
}

fn mean_rgb(img: &ove_render::Rgba) -> [f64; 3] {
    let mut s = [0.0; 3];
    for p in img.data.chunks(4) {
        for c in 0..3 {
            s[c] += p[c] as f64;
        }
    }
    s.map(|v| v / (img.data.len() / 4) as f64)
}

/// Playback reads the proxy when the setting is on and the proxy exists, and the original otherwise; export reads
/// the original whatever the setting, so its output is the same with proxies on and off.
#[test]
fn playback_uses_proxies_and_export_never_does() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("P.ove");
    std::fs::create_dir_all(bundle.join("proxies")).unwrap();
    let blue =
        make(dir.path(), "blue.mp4", &["-f", "lavfi", "-i", "color=c=blue:s=320x180:r=24:d=1", "-pix_fmt", "yuv420p"]);
    let red =
        make(dir.path(), "red.mp4", &["-f", "lavfi", "-i", "color=c=red:s=320x180:r=24:d=1", "-pix_fmt", "yuv420p"]);
    let cid = "0123456789abcdef0123456789abcdef";
    let on = clip_doc(&blue, cid, true);
    let off = clip_doc(&blue, cid, false);
    assert!(ove_render::proxy::enabled(&on) && !ove_render::proxy::enabled(&off));

    // No proxy yet: playback falls back to the original.
    let mut playback = Renderer::new(Some(&bundle), 180);
    playback.set_use_proxies(true);
    let c = mean_rgb(&playback.render(&on, 500_000));
    assert!(c[2] > 150.0 && c[0] < 60.0, "original (blue) while the proxy is missing: {c:?}");

    // A (deliberately different) proxy: playback with the setting on shows it.
    std::fs::copy(&red, proxy::path(&bundle, cid)).unwrap();
    let mut playback = Renderer::new(Some(&bundle), 180);
    playback.set_use_proxies(true);
    let c = mean_rgb(&playback.render(&on, 500_000));
    assert!(c[0] > 150.0 && c[2] < 60.0, "proxy (red) in playback: {c:?}");
    // Switched off, the same renderer reads the original again.
    playback.set_use_proxies(false);
    let c = mean_rgb(&playback.render(&on, 500_000));
    assert!(c[2] > 150.0, "original after switching off: {c:?}");

    // Export: identical frames with the setting on and off, and they are the original's.
    let settings = ExportSettings::h264(180, 24);
    let (a, b) = (dir.path().join("on.mp4"), dir.path().join("off.mp4"));
    export(&on, Some(&bundle), &a, &settings, &AtomicBool::new(false), |_| {}).unwrap();
    export(&off, Some(&bundle), &b, &settings, &AtomicBool::new(false), |_| {}).unwrap();
    let (fa, fb) = (frames_1080(&a, 100), frames_1080(&b, 100));
    assert_eq!(fa.len(), fb.len());
    assert!(fa.iter().zip(&fb).all(|(x, y)| x == y), "export frames differ with proxies on and off");
    let first = &fa[0].2;
    let blue_px = first.chunks(4).filter(|p| p[2] > 150 && p[0] < 60).count();
    assert!(blue_px > first.len() / 4 / 2, "the export shows the original (blue)");
}
