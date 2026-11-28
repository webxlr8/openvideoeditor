//! add-markers-keyframe-lanes-and-versions 3.4: `ove edit keyframes` sets, adds and removes one property's keyframes
//! in one revision each, and refuses keyframes outside the item with `keyframe.outOfRange`; the MCP tool
//! `edit_keyframes` has the same contract.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use serde_json::{Value, json};

fn ove(lib: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .arg("--json")
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn ok(o: &std::process::Output) -> Value {
    assert!(o.status.success(), "{} / {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

/// Project "Demo" with text item i_1 from 1 s to 4 s.
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    p.apply(
        vec![
            Op::AddTrack { track: json!({"id": "t", "name": "Main", "order": 0, "type": "video"}) },
            Op::AddItem {
                item: json!({"id": "i_1", "trackId": "t", "type": "text", "startTime": 1, "endTime": 4, "textContent": "Hi"}),
            },
        ],
        Source::Cli,
        None,
    )
    .unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn opacity(bundle: &Path) -> (Value, usize) {
    let p = Project::open(bundle, Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap();
    let t = ove_core::retime::find_track(p.doc().item("i_1").unwrap().fields.get("keyframeTracks"), "opacity").cloned();
    (t.unwrap_or(Value::Null), p.history().unwrap().len())
}

#[test]
fn cli_sets_adds_and_removes_keyframes_in_one_revision_each() {
    let (_d, lib, bundle) = setup();
    let (_, before) = opacity(&bundle);
    // The documented example.
    ok(&ove(&lib, &["edit", "keyframes", "Demo", "--item", "i_1", "--property", "opacity", "--set", "0:1,2:0"]));
    let (t, n) = opacity(&bundle);
    assert_eq!(n, before + 1, "one revision");
    assert_eq!(t["keyframes"], json!([{"time": 0, "value": 1.0}, {"time": 2, "value": 0.0}]));

    ok(&ove(&lib, &["edit", "keyframes", "Demo", "--item", "i_1", "--property", "opacity", "--add", "1:0.5:hold"]));
    let (t, n) = opacity(&bundle);
    assert_eq!(n, before + 2);
    assert_eq!(t["keyframes"][1], json!({"time": 1, "value": 0.5, "easing": "hold"}));

    ok(&ove(
        &lib,
        &[
            "edit",
            "keyframes",
            "Demo",
            "--item",
            "i_1",
            "--property",
            "opacity",
            "--remove",
            "1",
            "--easing",
            "ease-out",
        ],
    ));
    let (t, n) = opacity(&bundle);
    assert_eq!(n, before + 3);
    assert_eq!(t["keyframes"].as_array().unwrap().len(), 2);
    assert_eq!(t["easing"], "ease-out");

    // Crop edges keyframe too.
    ok(&ove(&lib, &["edit", "keyframes", "Demo", "--item", "i_1", "--property", "crop.left", "--set", "0:0,1:25"]));
}

#[test]
fn a_keyframe_outside_the_item_is_refused() {
    let (_d, lib, bundle) = setup();
    let (_, before) = opacity(&bundle);
    // The item is 3 s long: 8 s is 5 s after it ends.
    let o = ove(&lib, &["edit", "keyframes", "Demo", "--item", "i_1", "--property", "opacity", "--set", "0:1,8:0"]);
    assert_eq!(o.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(e["error"]["code"], "keyframe.outOfRange");
    assert_eq!(opacity(&bundle).1, before, "nothing changed");
}

#[test]
fn mcp_edit_keyframes_has_the_same_contract() {
    let (d, lib, bundle) = setup();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(&lib)
        .arg("mcp")
        .env_remove("OVE_LIBRARY")
        .env("OVE_CONFIG_DIR", d.path().join("config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut n = 0;
    let mut request = |method: &str, params: Value| -> Value {
        n += 1;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": n, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "agent"}}),
    );
    let set = request(
        "tools/call",
        json!({"name": "edit_keyframes", "arguments": {"project": "Demo", "item": "i_1", "property": "scale",
            "set": [{"time": 0, "value": 1, "easing": "ease-in"}, {"time": 3, "value": 1.5}]}}),
    );
    assert_eq!(set["result"]["isError"], false, "{set}");
    let far = request(
        "tools/call",
        json!({"name": "edit_keyframes", "arguments": {"project": "Demo", "item": "i_1", "property": "scale",
            "add": [{"time": 3.5, "value": 2}]}}),
    );
    assert_eq!(far["result"]["structuredContent"]["error"]["code"], "keyframe.outOfRange");
    let _ = child.kill();
    let _ = child.wait();
    let p = Project::open(&bundle, Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap();
    let t = ove_core::retime::find_track(p.doc().item("i_1").unwrap().fields.get("keyframeTracks"), "scale").unwrap();
    assert_eq!(t["keyframes"][0]["easing"], "ease-in");
    assert_eq!(t["keyframes"].as_array().unwrap().len(), 2);
}
