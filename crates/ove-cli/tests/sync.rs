//! add-audio-intelligence 2.4–2.5: `ove audio sync` (dry run, one revision, unrelated sound refused) and the
//! `sync_items` MCP contract, on synthetic speech-like recordings written as WAV files.
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

use ove_analyze::synth::{Recording, Scene};
use serde_json::{Value, json};

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

/// Project "Shoot": a camera recording on Main (item `cam`), a recorder that started 7.25 s later on Audio (`rec`,
/// placed at 50 s), and unrelated sound on Audio 2 (`other`).
fn setup(dir: &Path) -> PathBuf {
    let scene = Scene::speech(17);
    let files = [
        ("cam.wav", Recording::new(scene, 0.0, 30.0).noise(15.0, 1)),
        ("rec.wav", Recording::new(scene, 7.25, 20.0).lowpass(4_000.0).noise(10.0, 2)),
        ("other.wav", Recording::new(Scene::speech(71), 0.0, 20.0).noise(10.0, 3)),
    ];
    let mut paths = vec![];
    for (name, r) in &files {
        let p = dir.join(name);
        std::fs::write(&p, r.wav()).unwrap();
        paths.push(p);
    }
    let lib = dir.join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Shoot").unwrap();
    let mut ops = vec![];
    for (i, (id, track, kind, start, len)) in [
        ("cam", "Main", "video", 0.0, 30.0),
        ("rec", "Audio", "audio", 50.0, 20.0),
        ("other", "Audio 2", "audio", 0.0, 20.0),
    ]
    .iter()
    .enumerate()
    {
        let ttype = if *kind == "video" { "video" } else { "audio" };
        ops.push(
            json!({"op": "addTrack", "track": {"id": format!("t_{id}"), "name": track, "order": i, "type": ttype}}),
        );
        let mut entry =
            json!({"mediaId": format!("m_{id}"), "nativePath": paths[i].display().to_string(), "ownedByBundle": false});
        entry.as_object_mut().unwrap().extend(ove_store::media::identity_fields(&paths[i]));
        ops.push(json!({"op": "addMedia", "media": {"id": format!("m_{id}"), "data": {"type": kind, "fileName": format!("{id}.wav"), "duration": len, "hasAudio": true}},
            "entry": entry}));
        ops.push(json!({"op": "addItem", "item": {"id": id, "trackId": format!("t_{id}"), "type": kind, "startTime": start, "endTime": start + len, "mediaItemId": format!("m_{id}")}}));
    }
    let ops: Vec<ove_core::Op> = serde_json::from_value(Value::Array(ops)).unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    p.close().unwrap();
    lib
}

fn revisions(lib: &Path) -> usize {
    json_out(&ove(lib, &["history", "Shoot", "--json"]))["revisions"].as_array().unwrap().len()
}

fn item_start(lib: &Path, id: &str) -> f64 {
    let doc: Value = serde_json::from_slice(&std::fs::read(lib.join("Shoot.ove/project.json")).unwrap()).unwrap();
    let items = doc["project"]["timelineItems"].as_array().unwrap();
    items.iter().find(|i| i["id"] == id).unwrap()["startTime"].as_f64().unwrap()
}

#[test]
fn sync_dry_run_applies_as_one_revision_and_refuses_unrelated_sound() {
    let d = tempfile::tempdir().unwrap();
    let lib = setup(d.path());
    let before = revisions(&lib);

    let o = ove(&lib, &["audio", "sync", "Shoot", "--ref", "cam", "--targets", "rec", "--dry-run", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = json_out(&o);
    assert_eq!((v["dryRun"].as_bool(), v["placed"].as_u64()), (Some(true), Some(1)), "{v}");
    let t = &v["targets"][0];
    assert_eq!(
        (t["item"].as_str(), t["status"].as_str(), t["method"].as_str()),
        (Some("rec"), Some("placed"), Some("audio"))
    );
    let off = t["offsetSeconds"].as_f64().unwrap();
    assert!((off - 7.25).abs() <= 0.001, "offset {off}");
    assert!(t["confidence"].as_f64().unwrap() > 0.2, "{t}");
    assert!((t["start"].as_f64().unwrap() - 7.25).abs() <= 0.001, "{t}");
    assert_eq!(revisions(&lib), before, "a dry run creates no revision");
    assert_eq!(item_start(&lib, "rec"), 50.0);

    let o = ove(&lib, &["audio", "sync", "Shoot", "--ref", "cam", "--targets", "Audio", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(revisions(&lib), before + 1, "one revision");
    assert!((item_start(&lib, "rec") - 7.25).abs() <= 0.001);

    // Unrelated sound: not moved, non-zero exit, the result names the target and its confidence.
    let o = ove(&lib, &["audio", "sync", "Shoot", "--ref", "cam", "--targets", "other", "--json"]);
    assert_ne!(o.status.code(), Some(0));
    let e: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(e["error"]["code"], "sync.unmatched", "{e}");
    let t = &e["error"]["details"]["targets"][0];
    assert_eq!((t["item"].as_str(), t["status"].as_str()), (Some("other"), Some("unmatched")), "{e}");
    assert!(t["confidence"].is_number());
    assert_eq!(revisions(&lib), before + 1, "a refusal creates no revision");
    assert_eq!(item_start(&lib, "other"), 0.0);
}

#[test]
fn sync_items_answers_its_contract_over_mcp() {
    let d = tempfile::tempdir().unwrap();
    let lib = setup(d.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(&lib)
        .arg("mcp")
        .env_remove("OVE_LIBRARY")
        .env("OVE_MCP_HOLD_MS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut id = 0;
    let mut request = |method: &str, params: Value| -> Value {
        id += 1;
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        out.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    };
    request("initialize", json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "contract"}}));
    let tools = request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    let t = tools.iter().find(|t| t["name"] == "sync_items").expect("sync_items is listed");
    let props = &t["inputSchema"]["properties"];
    for k in ["reference", "targets", "mode", "window", "minConfidence", "drift", "dryRun"] {
        assert!(props.get(k).is_some(), "sync_items has `{k}`: {t}");
    }
    let req: Vec<&str> = t["inputSchema"]["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(req.contains(&"reference") && req.contains(&"targets"), "{t}");

    let r = request(
        "tools/call",
        json!({"name": "sync_items", "arguments": {"project": "Shoot", "reference": "cam", "targets": ["rec"], "dryRun": true}}),
    )["result"]
        .clone();
    assert_eq!(r["isError"], false, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["placed"], 1, "{s}");
    assert_eq!(s["targets"][0]["method"], "audio");
    assert!((s["targets"][0]["offsetSeconds"].as_f64().unwrap() - 7.25).abs() <= 0.001);

    let r = request(
        "tools/call",
        json!({"name": "sync_items", "arguments": {"project": "Shoot", "reference": "cam", "targets": "other"}}),
    )["result"]
        .clone();
    assert_eq!(r["isError"], true, "{r}");
    let _ = child.kill();
    let _ = child.wait();
}
