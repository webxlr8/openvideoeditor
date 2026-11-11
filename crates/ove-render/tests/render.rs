//! Compositor, audio and export on real (synthetic) media. Needs the ffmpeg CLI to make the media
//! (`OVE_FFMPEG_CLI` or `ffmpeg` on PATH); skipped with a message otherwise.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::{ExportSettings, Renderer, audio::Mixer, export};
use serde_json::{Value, json};

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FFMPEG_CLI").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffmpeg().unwrap()).args(["-loglevel", "error", "-y"]).args(args).arg(&out).status().unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    out
}

struct Fixture {
    _dir: tempfile::TempDir,
    doc: Document,
}

/// Main track: red [0,1) then blue [1,2) with a 0.5 s crossfade; a text item on top [0,2); an adjustment
/// (fully desaturated) over [2,3) on a green still.
fn fixture() -> Option<Fixture> {
    ffmpeg()?;
    let d = tempfile::tempdir().unwrap();
    let red = make(
        d.path(),
        "red.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x180:r=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let blue = make(
        d.path(),
        "blue.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:r=25",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let green = make(d.path(), "green.png", &["-f", "lavfi", "-i", "color=c=0x30c030:s=320x180", "-frames:v", "1"]);
    let media = |id: &str, p: &Path, kind: &str| Op::AddMedia {
        media: json!({"id": id, "data": {"type": kind, "fileName": p.file_name().unwrap().to_string_lossy(), "duration": 2, "hasAudio": kind == "video"}}),
        entry: Some(json!({"mediaId": id, "nativePath": p.display().to_string(), "ownedByBundle": false})),
    };
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "fx", "name": "FX", "order": 1, "type": "adjustment"}) },
        Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 2, "type": "video"}) },
        media("mr", &red, "video"),
        media("mb", &blue, "video"),
        media("mg", &green, "image"),
        Op::AddItem {
            item: json!({"id": "r", "trackId": "main", "type": "video", "startTime": 0, "endTime": 1, "mediaItemId": "mr"}),
        },
        Op::AddItem {
            item: json!({"id": "b", "trackId": "main", "type": "video", "startTime": 1, "endTime": 2, "mediaItemId": "mb", "transitionIn": {"type": "crossfade", "duration": 0.5}}),
        },
        Op::AddItem {
            item: json!({"id": "g", "trackId": "main", "type": "image", "startTime": 2, "endTime": 3, "mediaItemId": "mg"}),
        },
        Op::AddItem {
            item: json!({"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 0.8, "textContent": "HELLO", "fontSize": 200, "fontWeight": 700, "color": "#ffffff"}),
        },
        Op::AddItem {
            item: json!({"id": "a", "trackId": "fx", "type": "adjustment", "startTime": 2, "endTime": 3, "colorGrading": {"saturation": -1}}),
        },
    ];
    let doc = apply_batch(&Document::new("p", "Render", 1), &ops).unwrap().doc;
    Some(Fixture { _dir: d, doc })
}

fn near(a: [u8; 4], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

#[test]
fn composites_cuts_crossfades_text_and_adjustments() {
    let Some(f) = fixture() else { return eprintln!("skipped: no ffmpeg CLI") };
    let mut r = Renderer::new(None, 180);
    let frame = r.render(&f.doc, 900_000);
    assert_eq!((frame.w, frame.h), (320, 180));
    assert!(near(frame.px(10, 10), [255, 0, 0], 30), "red at 0.9 s: {:?}", frame.px(10, 10));
    // Crossfade midpoint: half red, half blue.
    let mid = r.render(&f.doc, 1_250_000).px(10, 10);
    assert!(near(mid, [128, 0, 128], 35), "crossfade midpoint {mid:?}");
    let after = r.render(&f.doc, 1_600_000).px(10, 10);
    assert!(near(after, [0, 0, 255], 30), "blue after the crossfade {after:?}");
    // Text: white ink in the middle while the text item is on screen.
    let with_text = r.render(&f.doc, 300_000);
    let white = with_text.data.chunks(4).filter(|p| p[0] > 230 && p[1] > 230 && p[2] > 230).count();
    assert!(white > 500, "text pixels {white}");
    // Adjustment desaturates the green still to grey.
    let graded = r.render(&f.doc, 2_500_000).px(160, 90);
    assert!(
        (graded[0] as i32 - graded[1] as i32).abs() <= 3 && (graded[1] as i32 - graded[2] as i32).abs() <= 3,
        "{graded:?}"
    );
    // Past the end: black.
    assert_eq!(r.render(&f.doc, 3_500_000).px(5, 5), [0, 0, 0, 255]);
}

#[test]
fn preview_render_speed() {
    let Some(f) = fixture() else { return };
    let mut r = Renderer::new(None, 540);
    let t = std::time::Instant::now();
    let n = 50;
    for i in 0..n {
        r.render(&f.doc, i * 40_000);
    }
    let fps = n as f64 / t.elapsed().as_secs_f64();
    eprintln!("PERF render 960x540 (1 video layer + text): {fps:.0} fps");
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|s| s * s).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

#[test]
fn audio_mix_levels_and_mutes() {
    let Some(f) = fixture() else { return };
    let mut m = Mixer::new(None);
    let base = rms(&m.mix(&f.doc, 200_000, 4800));
    assert!(base > 0.05, "sine audible: {base}");
    let louder = apply_batch(
        &f.doc,
        &[Op::UpdateItem { id: "r".into(), set: json!({"volume": 200}).as_object().unwrap().clone() }],
    )
    .unwrap()
    .doc;
    let mut m2 = Mixer::new(None);
    let l = rms(&m2.mix(&louder, 200_000, 4800));
    assert!((l / base - 2.0).abs() < 0.1, "200 % doubles the level: {base} → {l}");
    let muted = apply_batch(
        &f.doc,
        &[Op::UpdateTrack { id: "main".into(), set: json!({"muted": true}).as_object().unwrap().clone() }],
    )
    .unwrap()
    .doc;
    assert_eq!(rms(&Mixer::new(None).mix(&muted, 200_000, 4800)), 0.0);
    // Blue clip carries silence.
    assert!(rms(&Mixer::new(None).mix(&f.doc, 1_600_000, 4800)) < 1e-3);
    // Sequential blocks continue seamlessly (no restart clicks): consecutive reads equal one long read.
    let mut a = Mixer::new(None);
    let mut joined = a.mix(&f.doc, 100_000, 1024);
    joined.extend(a.mix(&f.doc, 100_000 + 1024 * 1_000_000 / 48_000, 1024));
    let one = Mixer::new(None).mix(&f.doc, 100_000, 2048);
    let diff = joined.iter().zip(&one).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
    assert!(diff < 0.02, "block joins differ by {diff}");
}

fn probe(path: &Path) -> Value {
    let bin = ffmpeg().unwrap().replace("ffmpeg", "ffprobe");
    let out = Command::new(bin)
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn export_round_trip() {
    let Some(f) = fixture() else { return };
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("out.mp4");
    let mut last = 0.0;
    let res = export(
        &f.doc,
        None,
        &out,
        &ExportSettings { height: 180, fps: 25, ..Default::default() },
        &AtomicBool::new(false),
        |p| last = p.fraction,
    )
    .unwrap();
    assert_eq!(last, 1.0);
    assert_eq!((res.width, res.height, res.frames), (320, 180, 75));
    eprintln!(
        "PERF export 320x180 3 s with {}: {:.2} s ({:.0}× real time)",
        res.video_encoder,
        res.seconds_taken,
        3.0 / res.seconds_taken
    );
    let p = probe(&out);
    let dur: f64 = p["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - 3.0).abs() < 0.1, "duration {dur}");
    let streams = p["streams"].as_array().unwrap();
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"));
    let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(320), Some(180)));
    // Frame at 1.7 s is blue.
    let raw = Command::new(ffmpeg().unwrap())
        .args(["-v", "error", "-ss", "1.7", "-i"])
        .arg(&out)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap()
        .stdout;
    let px = [raw[0], raw[1], raw[2], 255];
    assert!(near(px, [0, 0, 255], 40), "exported frame at 1.7 s: {px:?}");
    // Pending placeholders refuse export.
    let pending = apply_batch(
        &f.doc,
        &[Op::AddItem {
            item: json!({"id": "p", "trackId": "main", "type": "video", "startTime": 3, "endTime": 4, "pending": true}),
        }],
    )
    .unwrap()
    .doc;
    let e =
        export(&pending, None, &d.path().join("x.mp4"), &ExportSettings::default(), &AtomicBool::new(false), |_| {})
            .unwrap_err();
    assert!(e.to_string().contains("pending"), "{e}");
    // Cancel removes the partial file.
    let cancel = AtomicBool::new(true);
    let c = export(
        &f.doc,
        None,
        &d.path().join("c.mp4"),
        &ExportSettings { height: 180, fps: 25, ..Default::default() },
        &cancel,
        |_| {},
    );
    assert!(matches!(c, Err(ove_render::ExportError::Cancelled)));
    assert!(!d.path().join("c.mp4").exists());
}
