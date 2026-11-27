//! Save range as media and render in place (change extend-export-formats-and-queue, tasks 4.1–4.2): `ove export
//! --as-media` and MCP `save_range_as_media`. Needs `--features media`; text-only projects, so no media fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]
#![cfg(feature = "media")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use ove_core::model::Document;
use serde_json::{Value, json};

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

/// Project Title: a title [0, 14) on Text, a smaller one [3, 9) on More (above it), and audio track Music (empty).
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Title").unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "more", "name": "More", "order": 0, "type": "video"}},
        {"op": "addTrack", "track": {"id": "text", "name": "Text", "order": 1, "type": "video"}},
        {"op": "addTrack", "track": {"id": "music", "name": "Music", "order": 2, "type": "audio"}},
        {"op": "addItem", "item": {"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 14, "textContent": "MAIN TITLE", "fontSize": 150, "color": "#ffcc00"}},
        {"op": "addItem", "item": {"id": "u", "trackId": "more", "type": "text", "startTime": 3, "endTime": 9, "textContent": "lower third", "fontSize": 70, "y": 300}}
    ]))
    .unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn ove(lib: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .current_dir(cwd)
        .arg("--library")
        .arg(lib)
        .arg("--json")
        .args(args)
        .env_remove("OVE_LIBRARY")
        .env("OVE_EXPORT_QUEUE_DIR", cwd.join("queue"))
        .output()
        .unwrap()
}

fn ok_json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(String::from_utf8_lossy(&out.stdout).lines().last().unwrap()).unwrap()
}

fn doc(bundle: &Path) -> Document {
    ove_store::Project::open(bundle, ove_store::Mode::ReadOnly, clock()).unwrap().doc().clone()
}

/// (id, track, start, end) of every item, sorted.
fn items(d: &Document) -> Vec<(String, String, i64, i64)> {
    let mut v: Vec<_> = d.project.items.values().map(|i| (i.id.clone(), i.track_id.clone(), i.start, i.end)).collect();
    v.sort();
    v
}

fn rendered_files(bundle: &Path) -> Vec<String> {
    std::fs::read_dir(bundle.join("media").join("rendered"))
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

/// Mean SSIM of the luma of two equal-size RGBA frames, over 8×8 blocks.
fn ssim(a: &ove_render::Rgba, b: &ove_render::Rgba) -> f64 {
    assert_eq!((a.w, a.h), (b.w, b.h));
    let luma = |img: &ove_render::Rgba, x: u32, y: u32| {
        let p = img.px(x, y);
        0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
    };
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let (mut total, mut n) = (0.0, 0.0);
    for by in (0..a.h - 7).step_by(8) {
        for bx in (0..a.w - 7).step_by(8) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for y in by..by + 8 {
                for x in bx..bx + 8 {
                    let (p, q) = (luma(a, x, y), luma(b, x, y));
                    sa += p;
                    sb += q;
                    saa += p * p;
                    sbb += q * q;
                    sab += p * q;
                }
            }
            let k = 64.0;
            let (ma, mb) = (sa / k, sb / k);
            let (va, vb, cov) = (saa / k - ma * ma, sbb / k - mb * mb, sab / k - ma * mb);
            total += ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1.0;
        }
    }
    total / n
}

#[test]
fn a_range_saved_as_media_adds_one_owned_media_item_and_leaves_the_timeline() {
    let (d, lib, bundle) = setup();
    let before = doc(&bundle);
    let v = ok_json(&ove(&lib, d.path(), &["export", "Title", "--as-media", "--range", "5-12"]));
    let after = doc(&bundle);
    assert_eq!(items(&after), items(&before), "the timeline is unchanged");
    assert_eq!(after.project.media.len(), before.project.media.len() + 1);
    let id = v["mediaId"].as_str().unwrap();
    let m = after.project.media.get(id).unwrap();
    assert_eq!(m.fields.get("data").unwrap()["duration"], 7);
    let entry = after.registry.get(id).unwrap();
    let path = bundle.join(v["file"].as_str().unwrap());
    assert!(path.is_file());
    assert!(entry.owned_by_bundle && entry.native_path.starts_with("media/rendered/"), "{entry:?}");
    let info = ove_media::probe(&path).unwrap();
    let secs = info.duration_us.unwrap() as f64 / 1e6;
    assert!((secs - 7.0).abs() < 0.05, "{secs}");
    assert_eq!((info.video.as_ref().unwrap().width, info.video.as_ref().unwrap().height), (1920, 1080));
    eprintln!("RESULT saved 5–12 s as {} ({})", v["file"], v["videoFormat"]);
}

#[test]
fn render_in_place_replaces_the_covered_items_in_one_undoable_revision() {
    let (d, lib, bundle) = setup();
    let before = doc(&bundle);
    let mut r = ove_render::Renderer::new(Some(&bundle), 360);
    let frame_before = r.render(&before, 8_000_000);
    let v = ok_json(&ove(&lib, d.path(), &["export", "Title", "--as-media", "--range", "5-12", "--replace"]));
    assert_eq!(v["replaced"], true);
    assert_eq!(v["tracks"], json!(["more", "text"]), "every track but audio");
    let after = doc(&bundle);
    let clip = after.item(v["itemId"].as_str().unwrap()).unwrap();
    assert_eq!((clip.start, clip.end, clip.track_id.as_str()), (5_000_000, 12_000_000, "more"));
    // Nothing else is left inside 5–12 s on the replaced tracks.
    for i in after.project.items.values().filter(|i| i.id != clip.id) {
        assert!(i.end <= 5_000_000 || i.start >= 12_000_000, "{} still covers the range", i.id);
    }
    // The composite at 8 s looks the same.
    let mut r = ove_render::Renderer::new(Some(&bundle), 360);
    let frame_after = r.render(&after, 8_000_000);
    let s = ssim(&frame_before, &frame_after);
    assert!(s >= 0.98, "SSIM {s}");
    // One undo restores the items; the rendered media stays in the library.
    ok_json(&ove(&lib, d.path(), &["undo", "Title"]));
    assert_eq!(items(&doc(&bundle)), items(&before));
    assert_eq!(rendered_files(&bundle).len(), 1);
    eprintln!("RESULT render in place 5–12 s: SSIM at 8 s {s:.4}, one undo restores {} items", items(&before).len());
}

#[test]
fn a_failed_render_creates_no_revision_media_or_file() {
    let (d, lib, bundle) = setup();
    // A clip whose file is gone, inside the range.
    let mut p = ove_store::Project::open(&bundle, ove_store::Mode::Write, clock()).unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(json!([
        {"op": "addMedia", "media": {"id": "gone", "data": {"type": "video", "fileName": "vanished.mp4", "duration": 10, "hasAudio": true}},
         "entry": {"mediaId": "gone", "nativePath": d.path().join("vanished.mp4"), "ownedByBundle": false}},
        {"op": "addItem", "item": {"id": "v", "trackId": "text", "type": "video", "startTime": 14, "endTime": 20, "mediaItemId": "gone"}}
    ]))
    .unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    let rev = p.rev();
    p.close().unwrap();
    let out = ove(&lib, d.path(), &["export", "Title", "--as-media", "--range", "13-16", "--replace"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "export.refused");
    assert!(e["error"]["message"].as_str().unwrap().contains("vanished.mp4"), "{e}");
    let after = doc(&bundle);
    assert_eq!(after.project.rev, rev, "no revision");
    assert!(after.project.media.contains_key("gone") && after.project.media.len() == 1, "no media added");
    assert!(rendered_files(&bundle).is_empty(), "no file: {:?}", rendered_files(&bundle));
}

#[test]
fn mcp_save_range_as_media() {
    let (d, lib, bundle) = setup();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
        .current_dir(d.path())
        .arg("--library")
        .arg(&lib)
        .arg("mcp")
        .env_remove("OVE_LIBRARY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id: i64, args: Value| -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "save_range_as_media", "arguments": args}});
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()["result"].clone()
    };
    let r = call(
        1,
        json!({"project": "Title", "start": 1, "end": 2, "replace": true, "tracks": ["Text"], "small": true, "name": "Intro"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["tracks"], json!(["text"]));
    assert_eq!(s["file"], "media/rendered/Intro.mp4");
    // The More track has nothing at 1–2 s, so no transparency is needed.
    assert_eq!(s["alpha"], false);
    assert!(bundle.join("media/rendered/Intro.mp4").is_file());
    let bad = call(2, json!({"project": "Title", "start": 4, "end": 2}));
    assert_eq!(bad["isError"], true);
    assert_eq!(bad["structuredContent"]["error"]["code"], "export.invalid");
    let unknown = call(3, json!({"project": "Title", "start": 1, "end": 2, "tracks": ["Nope"]}));
    assert_eq!(unknown["structuredContent"]["error"]["code"], "track.unknown");
    let _ = child.kill();
    let _ = child.wait();
}
