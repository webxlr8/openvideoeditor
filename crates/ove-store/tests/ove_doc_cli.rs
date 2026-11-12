//! End-to-end script over every `ove-doc` command (task 6.2).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::process::{Command, Output};

use serde_json::{Value, json};

fn run(lib: &std::path::Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ove-doc"));
    c.arg("--library").arg(lib).arg("--json").args(args).env_remove("OVE_LIBRARY");
    if let Some(s) = stdin {
        use std::io::Write;
        c.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut ch = c.spawn().unwrap();
        ch.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        return ch.wait_with_output().unwrap();
    }
    c.output().unwrap()
}

fn ok(o: &Output) -> Value {
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

#[test]
fn every_command_end_to_end() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path();

    let created = ok(&run(lib, &["create", "Launch Film"], None));
    assert_eq!(created["name"], "Launch Film");

    let ops = json!([
        {"op": "addTrack", "track": {"id": "t1", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "t1", "type": "text", "startTime": 0, "endTime": 2, "textContent": "Hi"}}
    ]);
    let ops_file = d.path().join("ops.json");
    std::fs::write(&ops_file, ops.to_string()).unwrap();
    let r1 = ok(&run(lib, &["apply", "Launch Film", ops_file.to_str().unwrap(), "--source", "agent"], None));
    assert_eq!(r1["changed"], true);
    let r1_rev = r1["rev"].as_i64().unwrap();

    let r2 = ok(&run(lib, &["apply", "Launch Film", "-"], Some(r#"[{"op":"setAspectRatio","aspectRatio":"9:16"}]"#)));
    assert_eq!(r2["summary"], "1 aspect change");

    // Refused batch: exit 1 with a structured error on stderr and nothing changed.
    let bad = run(lib, &["apply", "Launch Film", "-"], Some(r#"[{"op":"removeTrack","id":"t1"}]"#));
    assert_eq!(bad.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&bad.stderr).unwrap();
    assert_eq!(err["error"]["code"], "item.track.exists");

    let list = ok(&run(lib, &["list"], None));
    assert_eq!(list[0]["name"], "Launch Film");
    assert_eq!(list[0]["aspectRatio"], "9:16");
    assert_eq!(list[0]["itemCount"], 1);

    let hist = ok(&run(lib, &["history", "Launch Film"], None));
    assert_eq!(hist.as_array().unwrap().len(), 2);
    assert_eq!(hist[0]["source"], "agent");

    let u = ok(&run(lib, &["undo", "Launch Film"], None));
    assert_eq!(u["source"], "undo");
    let shown = ok(&run(lib, &["show", "Launch Film"], None));
    assert_eq!(shown["project"]["aspectRatio"], "16:9");

    let r = ok(&run(lib, &["redo", "Launch Film"], None));
    assert_eq!(r["source"], "redo");
    let shown = ok(&run(lib, &["show", created["id"].as_str().unwrap()], None));
    assert_eq!(shown["project"]["aspectRatio"], "9:16");

    let rs = ok(&run(lib, &["restore", "Launch Film", &r1_rev.to_string()], None));
    assert_eq!(rs["source"], "restore");
    let shown = ok(&run(lib, &["show", "Launch Film"], None));
    assert_eq!(shown["project"]["aspectRatio"], "16:9");
    assert_eq!(shown["project"]["timelineItems"].as_array().unwrap().len(), 1);

    let missing = run(lib, &["show", "Nope"], None);
    assert_eq!(missing.status.code(), Some(1));
    let usage = run(lib, &["frobnicate"], None);
    assert_eq!(usage.status.code(), Some(2));
}
