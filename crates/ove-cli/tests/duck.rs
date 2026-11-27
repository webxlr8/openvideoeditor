//! `ove audio duck` (task 5.3): a dry run returns the planned keyframes and changes nothing, a run is one revision
//! that undo reverts, and refusals exit 1 with a code. Needs `--features media` and the ffmpeg CLI for fixtures
//! (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH; skipped with a message otherwise).
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
use std::sync::Arc;

use serde_json::{Value, json};

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

fn ove(lib: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn json_out(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|_| {
        panic!("stdout: {}\nstderr: {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    })
}

/// A project "Demo" with dialogue lines at 2–4 s and 6–7.5 s and 12 s of steady music.
fn setup(dir: &Path) -> std::path::PathBuf {
    let ff = ffmpeg().unwrap();
    let mut paths = vec![];
    for (name, expr) in [
        ("dialogue.wav", "0.3*sin(2*PI*1000*t)*(between(t\\,2\\,4)+between(t\\,6\\,7.5))"),
        ("music.wav", "0.2*sin(2*PI*440*t)"),
    ] {
        let out = dir.join(name);
        let src = format!("aevalsrc={expr}|{expr}:s=48000:d=12");
        let st = Command::new(&ff)
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &src, "-c:a", "pcm_s16le"])
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success());
        paths.push(out);
    }
    let lib = dir.join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let mut ops = vec![];
    for (i, (id, track)) in [("dlg", "Dialogue"), ("mus", "Music")].iter().enumerate() {
        ops.push(
            json!({"op": "addTrack", "track": {"id": format!("t_{id}"), "name": track, "order": i, "type": "audio"}}),
        );
        ops.push(json!({"op": "addMedia", "media": {"id": format!("m_{id}"), "data": {"type": "audio", "fileName": format!("{id}.wav"), "duration": 12}},
            "entry": {"mediaId": format!("m_{id}"), "nativePath": paths[i].display().to_string(), "ownedByBundle": false}}));
        ops.push(json!({"op": "addItem", "item": {"id": id, "trackId": format!("t_{id}"), "type": "audio", "startTime": 0, "endTime": 12, "mediaItemId": format!("m_{id}")}}));
    }
    let ops: Vec<ove_core::Op> = serde_json::from_value(Value::Array(ops)).unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    p.close().unwrap();
    lib
}

fn revisions(lib: &Path) -> usize {
    json_out(&ove(lib, &["history", "Demo", "--json"]))["revisions"].as_array().unwrap().len()
}

#[test]
fn duck_dry_run_apply_undo_and_refusals() {
    if ffmpeg().is_none() {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let lib = setup(d.path());
    let before = revisions(&lib);
    // Dry run: the plan, no revision.
    let o = ove(
        &lib,
        &["audio", "duck", "Demo", "--music", "Music", "--under", "Dialogue", "--depth", "-12", "--dry-run", "--json"],
    );
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = json_out(&o);
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["source"], "level");
    assert_eq!(v["speech"].as_array().unwrap().len(), 2);
    let keys = v["items"][0]["keyframes"].as_array().unwrap();
    assert!(keys.len() >= 8, "{keys:?}");
    assert_eq!(revisions(&lib), before, "a dry run creates no revision");
    // Apply: one revision with volume keyframes on the music item.
    let o = ove(&lib, &["audio", "duck", "Demo", "--music", "mus", "--under", "dlg", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(revisions(&lib), before + 1);
    let text = json_out(&ove(&lib, &["source", "print", "Demo", "--json"]))["text"].as_str().unwrap().to_string();
    assert!(text.contains("property: \"volume\""), "{text}");
    // Undo restores the music item without keyframes.
    assert_eq!(ove(&lib, &["undo", "Demo"]).status.code(), Some(0));
    let text = json_out(&ove(&lib, &["source", "print", "Demo", "--json"]))["text"].as_str().unwrap().to_string();
    assert!(!text.contains("property: \"volume\""), "{text}");
    // Refusals: an unknown id is named; no speech in silent dialogue.
    let n = revisions(&lib);
    let o = ove(&lib, &["audio", "duck", "Demo", "--music", "nope", "--under", "dlg", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(e["error"]["code"], "item.notFound");
    assert!(e["error"]["message"].as_str().unwrap().contains("nope"));
    let o =
        ove(&lib, &["audio", "duck", "Demo", "--music", "mus", "--under", "mus", "--source", "transcript", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(e["error"]["code"], "duck.noTranscript");
    assert_eq!(revisions(&lib), n, "refusals create no revision");
}
