//! Export formats (change extend-export-formats-and-queue, tasks 2.3–2.6): every codec path this computer's FFmpeg
//! build can write is exported from a 10 s timeline and checked with ffprobe (codec, profile, pixel format, colour
//! tags) and SSIM against the H.264 export; alpha, HDR tags, audio-only files and ranges are checked on their own.
//! Encoders missing from the build are reported as skipped, and their refusal message is checked instead.
//!
//! The media, ffprobe and SSIM come from the full ffmpeg CLI (`OVE_FIXTURE_FFMPEG` or `ffmpeg`, with `ffprobe` next
//! to it): without it the tests fail, unless `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` makes them print "skipped".
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
use ove_render::audio::{CHANNELS, Mixer, RATE};
use ove_render::export::pcm::{read_wav, to_s16, to_s24};
use ove_render::export::{
    AudioCodec, AudioSettings, Container, ExportError, ExportSettings, Fps, HDR_UNAVAILABLE, HdrTransfer, Inventory,
    Picture, Size, SourceSpec, VideoCodec, VideoSettings, export, export_frames, preset,
};
use ove_render::{Renderer, Rgba};
use serde_json::{Value, json};

fn ffmpeg() -> String {
    fixtures::fixture_ffmpeg()
}

/// ffprobe next to the fixture ffmpeg.
fn ffprobe() -> String {
    let ff = ffmpeg();
    let p = Path::new(&ff);
    let name = p.file_name().map(|n| n.to_string_lossy().replace("ffmpeg", "ffprobe")).unwrap_or("ffprobe".into());
    p.with_file_name(name).display().to_string()
}

fn tools() -> bool {
    fixtures::require_tool(&ffmpeg(), "it makes the export fixtures and measures SSIM; set OVE_FIXTURE_FFMPEG")
        && fixtures::require_tool(&ffprobe(), "it checks the exported files; it is looked for next to ffmpeg")
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffmpeg()).args(["-loglevel", "error", "-y"]).args(args).arg(&out).status().unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    out
}

/// ffprobe's JSON for a file's streams and format.
fn probe(p: &Path) -> Value {
    let o = Command::new(ffprobe())
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(p)
        .output()
        .unwrap();
    assert!(o.status.success(), "ffprobe {}: {}", p.display(), String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn stream<'a>(info: &'a Value, kind: &str) -> &'a Value {
    info["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == kind).unwrap_or(&Value::Null)
}

/// Mean SSIM of `b` against `a` in 4:2:0: (luma, all planes). Chroma is compared after a second subsampling for
/// 4:2:2 and 4:4:4 files, so luma is the measure of picture fidelity.
fn ssim(a: &Path, b: &Path) -> (f64, f64) {
    let o = Command::new(ffmpeg())
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(b)
        .arg("-i")
        .arg(a)
        .args(["-lavfi", "[0:v]format=yuv420p[x];[1:v]format=yuv420p[y];[x][y]ssim", "-f", "null", "-"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    let line = err.lines().rev().find(|l| l.contains("All:")).unwrap_or_else(|| panic!("no SSIM in {err}"));
    let value =
        |key: &str| -> f64 { line.split(key).nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap() };
    (value(" Y:"), value("All:"))
}

/// The first frame of a video file as straight RGBA.
fn first_frame_rgba(p: &Path, w: u32, h: u32) -> Vec<u8> {
    let o = Command::new(ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(p)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout.len(), (w * h * 4) as usize, "decoded frame size");
    o.stdout
}

struct Fixture {
    dir: tempfile::TempDir,
    doc: Document,
}

/// 10 s: a moving test pattern with a tone, a title over its first 4 s.
fn fixture(secs: u32) -> Option<Fixture> {
    if !tools() {
        return None;
    }
    let d = tempfile::tempdir().unwrap();
    let t = secs.to_string();
    let clip = make(
        d.path(),
        "pattern.mov",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:sample_rate=48000",
            "-t",
            &t,
            "-c:v",
            "mjpeg",
            "-q:v",
            "2",
            "-c:a",
            "pcm_s16le",
            "-ac",
            "2",
        ],
    );
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 1, "type": "video"}) },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "video", "fileName": "pattern.mov", "duration": secs, "hasAudio": true}}),
            entry: Some(json!({"mediaId": "m", "nativePath": clip.display().to_string(), "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "v", "trackId": "main", "type": "video", "startTime": 0, "endTime": secs, "mediaItemId": "m"}),
        },
        Op::AddItem {
            item: json!({"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 4, "textContent": "EXPORT", "fontSize": 160, "fontWeight": 700, "color": "#ffffff"}),
        },
    ];
    let doc = apply_batch(&Document::new("p", "Formats", 1), &ops).unwrap().doc;
    Some(Fixture { dir: d, doc })
}

fn video(codec: VideoCodec, profile: Option<&str>, depth: Option<u8>) -> VideoSettings {
    VideoSettings { codec, profile: profile.map(str::to_string), bit_depth: depth, ..VideoSettings::default() }
}

fn settings(container: Container, v: VideoSettings, audio: AudioCodec) -> ExportSettings {
    ExportSettings {
        container,
        video: Some(v),
        audio: Some(AudioSettings { codec: audio, ..AudioSettings::default() }),
        size: Size::Height(360),
        fps: Fps::Rate(24),
        ..ExportSettings::default()
    }
}

struct Case {
    name: &'static str,
    settings: ExportSettings,
    /// ffprobe codec_name, profile (when checked), pix_fmt.
    codec: &'static str,
    profile: Option<&'static str>,
    pix_fmts: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    use VideoCodec::*;
    let prores = |name, p, prof, fmt| Case {
        name,
        settings: settings(Container::Mov, video(Prores, Some(p), None), AudioCodec::Pcm),
        codec: "prores",
        profile: Some(prof),
        pix_fmts: fmt,
    };
    let dnx = |name, p, prof, fmt| Case {
        name,
        settings: settings(Container::Mov, video(Dnxhr, Some(p), None), AudioCodec::Pcm),
        codec: "dnxhd",
        profile: Some(prof),
        pix_fmts: fmt,
    };
    vec![
        Case {
            name: "HEVC 8-bit",
            settings: settings(Container::Mp4, video(Hevc, None, Some(8)), AudioCodec::Aac),
            codec: "hevc",
            profile: Some("Main"),
            pix_fmts: &["yuv420p", "nv12"],
        },
        Case {
            name: "HEVC 10-bit",
            settings: settings(Container::Mp4, video(Hevc, None, Some(10)), AudioCodec::Aac),
            codec: "hevc",
            profile: Some("Main 10"),
            pix_fmts: &["yuv420p10le", "p010le"],
        },
        prores("ProRes 422 Proxy", "proxy", "Proxy", &["yuv422p10le"]),
        prores("ProRes 422 LT", "lt", "LT", &["yuv422p10le"]),
        prores("ProRes 422", "standard", "Standard", &["yuv422p10le"]),
        prores("ProRes 422 HQ", "hq", "HQ", &["yuv422p10le"]),
        prores("ProRes 4444", "4444", "4444", &["yuv444p10le", "yuv444p12le"]),
        dnx("DNxHR LB", "lb", "DNXHR LB", &["yuv422p"]),
        dnx("DNxHR SQ", "sq", "DNXHR SQ", &["yuv422p"]),
        dnx("DNxHR HQ", "hq", "DNXHR HQ", &["yuv422p"]),
        dnx("DNxHR HQX", "hqx", "DNXHR HQX", &["yuv422p10le"]),
        dnx("DNxHR 444", "444", "DNXHR 444", &["yuv444p10le", "gbrp10le"]),
        Case {
            name: "VP9",
            settings: settings(Container::Webm, video(Vp9, None, None), AudioCodec::Opus),
            codec: "vp9",
            profile: None,
            pix_fmts: &["yuv420p"],
        },
        Case {
            name: "AV1 (hardware)",
            settings: settings(Container::Mp4, video(Av1, None, None), AudioCodec::Aac),
            codec: "av1",
            profile: None,
            pix_fmts: &["yuv420p", "nv12"],
        },
    ]
}

#[test]
fn codec_matrix_probes_as_expected_and_matches_the_h264_export() {
    let Some(f) = fixture(10) else { return };
    let never = AtomicBool::new(false);
    let mut h264 = settings(Container::Mp4, video(VideoCodec::H264, None, None), AudioCodec::Aac);
    h264.video.as_mut().unwrap().quality = ove_render::export::Quality::VeryHigh;
    let golden = f.dir.path().join("golden.mp4");
    let g = export(&f.doc, None, &golden, &h264, &never, |_| {}).unwrap();
    eprintln!("RESULT H.264 golden: {:?}, {:.1}x real time", g.video_encoder, 10.0 / g.seconds_taken);
    let mut skipped = Vec::new();
    for case in cases() {
        let out = f.dir.path().join(format!("{}.{}", case.name.replace(' ', "_"), case.settings.container.extension()));
        let r = export(&f.doc, None, &out, &case.settings, &never, |_| {});
        let r = match r {
            Ok(r) => r,
            Err(e @ (ExportError::Invalid(_) | ExportError::Refused(_))) => {
                // Only a missing or unusable encoder may skip a case; the message names the codec.
                let msg = e.to_string();
                let label = case.settings.video.as_ref().unwrap().codec.label();
                assert!(msg.contains(label) || msg.contains("cannot write"), "{}: {msg}", case.name);
                eprintln!("SKIPPED {}: {msg}", case.name);
                skipped.push(case.name);
                assert!(!out.exists());
                continue;
            }
            Err(e) => panic!("{}: {e}", case.name),
        };
        let info = probe(&out);
        let v = stream(&info, "video");
        assert_eq!(v["codec_name"], case.codec, "{}: {v}", case.name);
        if let Some(p) = case.profile {
            assert_eq!(v["profile"].as_str(), Some(p), "{}: profile {v}", case.name);
        }
        let fmt = v["pix_fmt"].as_str().unwrap_or_default();
        assert!(case.pix_fmts.contains(&fmt), "{}: pix_fmt {fmt}", case.name);
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(640), Some(360)), "{}", case.name);
        assert_eq!(v["color_primaries"], "bt709", "{}: {v}", case.name);
        assert_eq!(v["color_transfer"], "bt709", "{}: {v}", case.name);
        assert_eq!(v["color_space"], "bt709", "{}: {v}", case.name);
        assert!(!stream(&info, "audio").is_null(), "{}: no audio stream", case.name);
        let (s, all) = ssim(&golden, &out);
        let speed = 10.0 / r.seconds_taken;
        eprintln!(
            "RESULT {}: encoder {:?} ({:?}), fallbacks {:?}, SSIM luma {s:.4} all {all:.4}, {speed:.1}x real time at 640x360",
            case.name, r.video_encoder, r.encoder_label, r.encoder_fallbacks
        );
        assert!(r.video_encoder.is_some() && r.encoder_label.is_some());
        // Proxy is the offline-editing profile, at about a fifth of HQ's data rate: its floor is lower.
        let floor = if case.name.contains("Proxy") { 0.98 } else { 0.99 };
        assert!(s >= floor, "{}: SSIM {s} against the H.264 export", case.name);
    }
    eprintln!("skipped (no encoder in this build or on this computer): {skipped:?}");
}

#[test]
fn prores_master_preset_writes_prores_hq_and_24_bit_pcm_at_the_timeline_size() {
    let Some(f) = fixture(2) else { return };
    let out = f.dir.path().join("master.mov");
    let r =
        export(&f.doc, None, &out, &preset("ProRes 422 HQ master").unwrap().settings, &AtomicBool::new(false), |_| {})
            .unwrap();
    let info = probe(&out);
    let v = stream(&info, "video");
    assert_eq!((v["codec_name"].as_str(), v["profile"].as_str()), (Some("prores"), Some("HQ")));
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(1920), Some(1080)), "match timeline size");
    assert_eq!(v["pix_fmt"], "yuv422p10le");
    let a = stream(&info, "audio");
    assert_eq!(a["codec_name"], "pcm_s24le");
    assert!(r.encoder_label.is_some());
    eprintln!("RESULT ProRes 422 HQ master 1080p 2 s: {:.2} s, encoder {:?}", r.seconds_taken, r.encoder_label);
}

/// A timeline with only a text item.
fn text_only() -> Document {
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddItem {
            item: json!({"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 1, "textContent": "ALPHA", "fontSize": 220, "fontWeight": 700, "color": "#ffcc00"}),
        },
    ];
    apply_batch(&Document::new("p", "Alpha", 1), &ops).unwrap().doc
}

fn check_alpha(out: &Path, expect: &Rgba) {
    let got = first_frame_rgba(out, expect.w, expect.h);
    let (mut outside, mut inside) = (0usize, 0usize);
    for (i, px) in expect.data.chunks_exact(4).enumerate() {
        let a = got[i * 4 + 3];
        match px[3] {
            0 => {
                assert_eq!(a, 0, "pixel {i} outside the text must be transparent");
                outside += 1;
            }
            255 => {
                assert_eq!(a, 255, "pixel {i} inside the text must be opaque");
                inside += 1;
            }
            _ => {}
        }
    }
    assert!(outside > 1000 && inside > 1000, "outside {outside}, inside {inside}");
    eprintln!("alpha checked: {outside} transparent and {inside} opaque pixels");
}

#[test]
fn prores_4444_and_vp9_carry_the_alpha_of_a_text_only_timeline() {
    if !tools() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let doc = text_only();
    let mut r = Renderer::new(None, 360);
    r.set_transparent(true);
    let expect = r.render(&doc, 0);
    let mut s = preset("ProRes 4444 with alpha").unwrap().settings.clone();
    s.size = Size::Height(360);
    let out = d.path().join("alpha.mov");
    let res = export(&doc, None, &out, &s, &AtomicBool::new(false), |_| {}).unwrap();
    let info = probe(&out);
    assert!(
        ["yuva444p10le", "yuva444p12le"].contains(&stream(&info, "video")["pix_fmt"].as_str().unwrap()),
        "alpha pixel format"
    );
    assert_eq!(stream(&info, "video")["profile"], "4444");
    check_alpha(&out, &expect);
    eprintln!("RESULT ProRes 4444 alpha via {:?}", res.video_encoder);

    let mut s = preset("Web VP9 with alpha").unwrap().settings.clone();
    s.size = Size::Height(360);
    let out = d.path().join("alpha.webm");
    match export(&doc, None, &out, &s, &AtomicBool::new(false), |_| {}) {
        Ok(_) => check_alpha(&out, &expect),
        Err(e) => {
            let inv = Inventory::current();
            assert!(!inv.has_encoder("libvpx-vp9") || !inv.has_muxer("webm"), "{e}");
            assert!(e.to_string().contains("VP9") || e.to_string().contains("WebM"), "{e}");
            eprintln!("SKIPPED VP9 alpha: {e}");
        }
    }
}

#[test]
fn hdr_is_refused_for_sdr_timelines_and_tagged_for_hdr_sources() {
    if !tools() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let doc = text_only();
    let hdr = |codec, depth| ExportSettings {
        container: Container::Mov,
        video: Some(video(codec, None, depth)),
        audio: None,
        hdr: Some(HdrTransfer::Pq),
        size: Size::Height(360),
        fps: Fps::Rate(24),
        ..ExportSettings::default()
    };
    // The timeline compositor gives SDR frames only: refused, no file.
    let out = d.path().join("hdr.mov");
    let e = export(&doc, None, &out, &hdr(VideoCodec::Prores, None), &AtomicBool::new(false), |_| {}).unwrap_err();
    assert_eq!(e.to_string(), HDR_UNAVAILABLE);
    assert!(!out.exists());

    // A synthetic HDR source: a PQ-coded BT.2020 ramp in 16-bit RGBA.
    let (w, h) = (320u32, 180u32);
    let frame = |_t: i64| {
        let mut data = Vec::with_capacity((w * h * 8) as usize);
        for _y in 0..h {
            for x in 0..w {
                let v = (x * 65_535 / (w - 1)) as u16;
                for c in [v, v / 2, 65_535 - v, 65_535] {
                    data.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        Picture::Rgba16 { w, h, data }
    };
    let spec = SourceSpec { width: w, height: h, fps: 24, duration: 500_000, hdr: true };
    for (codec, depth, name) in
        [(VideoCodec::Prores, None, "hdr_prores.mov"), (VideoCodec::Hevc, Some(10), "hdr_hevc.mov")]
    {
        let out = d.path().join(name);
        let r = export_frames(
            spec,
            &hdr(codec, depth),
            &out,
            frame,
            |_, n| vec![0.0; n * CHANNELS],
            &AtomicBool::new(false),
            |_| {},
        );
        let r = match r {
            Ok(r) => r,
            Err(e) if codec == VideoCodec::Hevc => {
                eprintln!("SKIPPED HEVC 10-bit PQ: {e}");
                continue;
            }
            Err(e) => panic!("{name}: {e}"),
        };
        let o = Command::new(ffprobe())
            .args(["-v", "error", "-select_streams", "v:0", "-show_streams", "-of", "json"])
            .arg(&out)
            .output()
            .unwrap();
        let info: Value = serde_json::from_slice(&o.stdout).unwrap();
        let v = &info["streams"][0];
        assert_eq!(v["color_primaries"], "bt2020", "{name}: {v}");
        assert_eq!(v["color_transfer"], "smpte2084", "{name}: {v}");
        assert_eq!(v["color_space"], "bt2020nc", "{name}: {v}");
        assert!(v["pix_fmt"].as_str().unwrap().contains("10"), "{name}: 10-bit {v}");
        let side = v["side_data_list"].as_array().cloned().unwrap_or_default();
        let types: Vec<&str> = side.iter().filter_map(|s| s["side_data_type"].as_str()).collect();
        assert!(types.iter().any(|t| t.contains("Mastering display")), "{name}: side data {types:?}");
        assert!(types.iter().any(|t| t.contains("Content light")), "{name}: side data {types:?}");
        eprintln!("RESULT HDR {name} via {:?}: {types:?}", r.video_encoder);
    }
}

/// An audio-only timeline: a stereo tone of `secs`.
fn tone_doc(dir: &Path, secs: u32) -> Document {
    let wave = "0.5*sin(2*PI*440*t)|0.3*sin(2*PI*660*t)";
    let path = make(
        dir,
        "tone.wav",
        &["-f", "lavfi", "-i", &format!("aevalsrc={wave}:s=48000:d={secs}"), "-c:a", "pcm_s24le"],
    );
    let ops = vec![
        Op::AddTrack { track: json!({"id": "a", "name": "Audio", "order": 0, "type": "audio"}) },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "audio", "fileName": "tone.wav", "duration": secs, "hasAudio": true}}),
            entry: Some(json!({"mediaId": "m", "nativePath": path.display().to_string(), "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "x", "trackId": "a", "type": "audio", "startTime": 0, "endTime": secs, "mediaItemId": "m", "volume": 80}),
        },
    ];
    apply_batch(&Document::new("p", "Tone", 1), &ops).unwrap().doc
}

/// The mixer's output from `t0` for `n` sample frames, in the blocks audio-only export uses (0.1 s).
fn mix(doc: &Document, t0: i64, n: usize) -> Vec<f32> {
    let mut m = Mixer::new(None);
    let mut out = Vec::with_capacity(n * CHANNELS);
    let mut pos = 0usize;
    while pos < n {
        let k = 4800.min(n - pos);
        out.extend(m.mix(doc, t0 + pos as i64 * 1_000_000 / RATE as i64, k));
        pos += k;
    }
    out
}

#[test]
fn wav_matches_the_mix_and_m4a_keeps_the_duration() {
    if !tools() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let doc = tone_doc(d.path(), 3);
    for (name, depth) in [("Audio WAV 24-bit", 24u8), ("Audio WAV 16-bit", 16)] {
        let out = d.path().join(format!("mix{depth}.wav"));
        let r = export(&doc, None, &out, &preset(name).unwrap().settings, &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!((r.width, r.frames, r.video_encoder.as_deref()), (0, 0, None));
        let (rate, ch, got_depth, got) = read_wav(&out).unwrap();
        assert_eq!((rate, ch as usize, got_depth), (RATE, CHANNELS, depth));
        let want = mix(&doc, 0, 3 * RATE as usize);
        assert_eq!(got.len(), want.len(), "{name}: sample count");
        let worst = got
            .iter()
            .zip(&want)
            .map(|(g, w)| (g - if depth == 24 { to_s24(*w) } else { i32::from(to_s16(*w)) }).abs())
            .max()
            .unwrap();
        assert!(worst <= 1, "{name}: off by {worst} LSB");
        let info = probe(&out);
        assert_eq!(stream(&info, "audio")["codec_name"], if depth == 24 { "pcm_s24le" } else { "pcm_s16le" });
        eprintln!("RESULT {name}: max difference from the mix {worst} LSB");
    }
    // AAC in M4A: the priming delay is in the edit list, so the duration is the timeline's within one AAC frame.
    let out = d.path().join("mix.m4a");
    export(&doc, None, &out, &preset("Audio M4A").unwrap().settings, &AtomicBool::new(false), |_| {}).unwrap();
    let info = probe(&out);
    let dur: f64 = info["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - 3.0).abs() <= 1024.0 / 48_000.0, "M4A duration {dur}");
    assert_eq!(stream(&info, "audio")["codec_name"], "aac");
    assert!(stream(&info, "video").is_null());
    eprintln!("RESULT M4A duration {dur:.4} s for a 3 s timeline");
    // FLAC, when the build has the encoder.
    let out = d.path().join("mix.flac");
    match export(&doc, None, &out, &preset("Audio FLAC").unwrap().settings, &AtomicBool::new(false), |_| {}) {
        Ok(_) => assert_eq!(stream(&probe(&out), "audio")["codec_name"], "flac"),
        Err(e) => {
            assert!(!Inventory::current().has_encoder("flac") || !Inventory::current().has_muxer("flac"), "{e}");
            eprintln!("SKIPPED FLAC: {e}");
        }
    }
}

#[test]
fn a_range_exports_only_that_part() {
    let Some(f) = fixture(6) else { return };
    let never = AtomicBool::new(false);
    let mut s = preset("ProRes 422 HQ master").unwrap().settings.clone();
    s.size = Size::Height(360);
    s.range = Some([2.0, 4.0]);
    let out = f.dir.path().join("range.mov");
    let r = export(&f.doc, None, &out, &s, &never, |_| {}).unwrap();
    assert_eq!(r.range, Some([2.0, 4.0]));
    let info = probe(&out);
    let dur: f64 = info["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - 2.0).abs() < 0.05, "duration {dur}");
    // The first frame is the timeline at 2 s.
    let expect = Renderer::new(None, 360).render(&f.doc, 2_000_000);
    let got = first_frame_rgba(&out, expect.w, expect.h);
    let diff: f64 = got
        .chunks_exact(4)
        .zip(expect.data.chunks_exact(4))
        .map(|(g, e)| (0..3).map(|c| (g[c] as f64 - e[c] as f64).abs()).sum::<f64>() / 3.0)
        .sum::<f64>()
        / (expect.w * expect.h) as f64;
    assert!(diff < 3.0, "first frame differs from the timeline at 2 s by {diff:.2} per channel");
    eprintln!("RESULT range 2–4 s: duration {dur:.3} s, first-frame mean difference {diff:.2}");
    // Audio of a range: the WAV holds the mix from 2 s.
    let mut s = preset("Audio WAV 24-bit").unwrap().settings.clone();
    s.range = Some([2.0, 4.0]);
    let out = f.dir.path().join("range.wav");
    export(&f.doc, None, &out, &s, &never, |_| {}).unwrap();
    let (_, _, _, got) = read_wav(&out).unwrap();
    let want = mix(&f.doc, 2_000_000, 2 * RATE as usize);
    assert_eq!(got.len(), want.len());
    assert!(got.iter().zip(&want).all(|(g, w)| (g - to_s24(*w)).abs() <= 1));
    // A reversed range is refused before anything is written.
    s.range = Some([4.0, 2.0]);
    let out = f.dir.path().join("bad.wav");
    let e = export(&f.doc, None, &out, &s, &never, |_| {}).unwrap_err();
    assert!(matches!(e, ExportError::Invalid(_)) && e.to_string().contains("after its start"), "{e}");
    assert!(!out.exists() && !ove_render::export::partial_path(&out).exists());
}

#[test]
fn match_timeline_size_on_a_vertical_project() {
    let doc = {
        let mut d = text_only();
        d = apply_batch(&d, &[Op::SetAspectRatio { aspect_ratio: "9:16".into() }]).unwrap().doc;
        d
    };
    let p = ove_render::export::prepare(
        &doc,
        None,
        &ExportSettings { size: Size::MatchTimeline, ..ExportSettings::default() },
    )
    .unwrap();
    assert_eq!((p.width, p.height), (1080, 1920));
}
