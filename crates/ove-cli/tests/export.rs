//! `ove export`: the examples in docs/cli/export.md run with their documented exit codes, the files they write are
//! what the settings say, and every preset is documented. Needs `--features media`; a text-only project, so no media
//! fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]
#![cfg(feature = "media")]

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn ove(lib: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .current_dir(cwd)
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        // A queue of its own, not the app data folder's.
        .env("OVE_EXPORT_QUEUE_DIR", cwd.join("queue"))
        .output()
        .unwrap()
}

/// A library with project Title: a 2 s text item.
fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, std::sync::Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Title").unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "text", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 2, "textContent": "Title", "fontSize": 160}}
    ]))
    .unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    p.close().unwrap();
    (d, lib)
}

fn docs() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli/export.md")).unwrap()
}

#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let (d, lib) = setup();
    let mut in_sh = false;
    let mut ran = 0;
    for line in docs().lines() {
        if line.starts_with("```") {
            in_sh = line == "```sh";
            continue;
        }
        if !in_sh || !line.starts_with("ove ") {
            continue;
        }
        let (cmd, comment) = line.split_once('#').expect("example has an `# exit N` comment");
        let want: i32 = comment.trim().strip_prefix("exit ").unwrap().parse().unwrap();
        let mut args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
        args.push("--json");
        let out = ove(&lib, d.path(), &args);
        assert_eq!(
            out.status.code(),
            Some(want),
            "`{}`\nstdout: {}\nstderr: {}",
            cmd.trim(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if want == 0 {
            let last = String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or_default().to_string();
            let v: Value = serde_json::from_str(&last).unwrap();
            assert!(
                v["path"].is_string() || v["jobId"].is_string() || v["jobs"].is_array() || v["presets"].is_array(),
                "{last}"
            );
        } else {
            let e: Value = serde_json::from_slice(&out.stderr).unwrap();
            let code = e["error"]["code"].as_str().unwrap();
            assert!(["export.invalid", "export.preset.unknown", "job.unknown"].contains(&code), "{e}");
        }
        ran += 1;
    }
    assert_eq!(ran, 19);
    // The detached export finishes on its own.
    let q = ove_render::queue::Queue::new(d.path().join("queue"));
    for j in q.list() {
        let done = q.wait(&j.id, std::time::Duration::from_secs(300), |_| {}).unwrap();
        assert!(done.status.finished(), "{done:?}");
    }
    assert!(d.path().join("later.mov").is_file());
    let dir = d.path();
    // The refused ones wrote nothing.
    for f in ["wrong.mp4", "wrong.mov"] {
        assert!(!dir.join(f).exists(), "{f}");
    }
    let probe = |f: &str| ove_media::probe(&dir.join(f)).unwrap();
    let master = probe("master.mov");
    let v = master.video.unwrap();
    assert_eq!((v.codec.as_str(), v.width, v.height), ("prores", 1920, 1080));
    assert_eq!(master.audio.unwrap().codec, "pcm_s24le");
    let overlay = probe("overlay.mov").video.unwrap();
    assert!(overlay.pixel_format.unwrap().has_alpha, "ProRes 4444 with alpha");
    assert_eq!(overlay.height, 720);
    let middle = probe("middle.mov");
    let secs = middle.duration_us.unwrap() as f64 / 1e6;
    assert!((secs - 1.0).abs() < 0.05, "range 0.5–1.5 s lasts {secs} s");
    let m4a = probe("mix.m4a");
    assert!(m4a.video.is_none() && m4a.audio.unwrap().codec == "aac");
    assert_eq!(std::fs::read(dir.join("mix.wav")).unwrap()[..4], *b"RIFF");
}

#[test]
fn every_preset_is_documented() {
    let text = docs();
    for p in ove_render::export::presets() {
        assert!(text.contains(&format!("| {} |", p.name)), "docs/cli/export.md lacks preset {}", p.name);
    }
}
