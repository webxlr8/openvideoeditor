//! add-markers-keyframe-lanes-and-versions 2.3: `ove markers list|add|update|remove` with status and assignee filters,
//! the MCP marker tools returning the items under each marker, and the refusal of `resolved` from an agent.
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

/// A library with project "Demo": one text item from 0 s to 10 s and one from 10 s to 20 s.
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let track = "t_main";
    p.apply(
        vec![
            Op::AddTrack { track: json!({"id": track, "name": "Main", "order": 0, "type": "video"}) },
            Op::AddItem {
                item: json!({"id": "i_a", "trackId": track, "type": "text", "startTime": 0, "endTime": 10, "textContent": "A"}),
            },
            Op::AddItem {
                item: json!({"id": "i_b", "trackId": track, "type": "text", "startTime": 10, "endTime": 20, "textContent": "B"}),
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

#[test]
fn cli_adds_lists_filters_updates_and_removes_markers() {
    let (_d, lib, _) = setup();
    let a = ok(&ove(&lib, &["markers", "add", "Demo", "--at", "12", "--name", "Tighten", "--assignee", "agent"]));
    let m1 = a["marker"]["id"].as_str().unwrap().to_string();
    assert_eq!(a["marker"]["status"], "open");
    assert_eq!(a["marker"]["items"], json!(["i_b"]));
    let b = ok(&ove(&lib, &["markers", "add", "Demo", "--at", "8", "--dur", "4", "--status", "review"]));
    let m2 = b["marker"]["id"].as_str().unwrap().to_string();
    assert_eq!(b["marker"]["items"], json!(["i_a", "i_b"]), "a range marker lists every item it overlaps");

    let all = ok(&ove(&lib, &["markers", "list", "Demo"]));
    let ids: Vec<&str> = all["markers"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [m2.as_str(), m1.as_str()], "time order");
    let open = ok(&ove(&lib, &["markers", "list", "Demo", "--status", "open", "--assignee", "agent"]));
    assert_eq!(open["markers"].as_array().unwrap().len(), 1);
    assert_eq!(open["markers"][0]["id"], m1.as_str());
    let none = ok(&ove(&lib, &["markers", "list", "Demo", "--assignee", "user"]));
    assert!(none["markers"].as_array().unwrap().is_empty());

    // A person on the command line may resolve a marker.
    let u = ok(&ove(&lib, &["markers", "update", "Demo", &m1, "--status", "resolved", "--unset", "assignee"]));
    assert_eq!(u["marker"]["status"], "resolved");
    assert!(u["marker"].get("assignee").is_none());

    let bad = ove(&lib, &["markers", "update", "Demo", &m1, "--status", "done"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("argument.invalid"));

    ok(&ove(&lib, &["markers", "remove", "Demo", &m1, &m2]));
    assert!(ok(&ove(&lib, &["markers", "list", "Demo"]))["markers"].as_array().unwrap().is_empty());
    let gone = ove(&lib, &["markers", "remove", "Demo", &m1]);
    assert_eq!(gone.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&gone.stderr).contains("marker.missing"));
}

#[test]
fn ripple_edits_carry_markers_when_asked() {
    let (_d, lib, _) = setup();
    let m = ok(&ove(&lib, &["markers", "add", "Demo", "--at", "15"]))["marker"]["id"].as_str().unwrap().to_string();
    let at = |lib: &Path| ok(&ove(lib, &["markers", "list", "Demo"]))["markers"][0]["at"].as_f64().unwrap();
    ok(&ove(&lib, &["edit", "ripple-trim", "Demo", "--item", "i_a", "--edge", "end", "--to", "9"]));
    assert_eq!(at(&lib), 15.0, "markers stay without --markers");
    ok(&ove(&lib, &["edit", "ripple-trim", "Demo", "--item", "i_a", "--edge", "end", "--to", "8", "--markers"]));
    assert_eq!(at(&lib), 14.0, "with --markers the marker moves with the ripple");
    let _ = m;
}

/// The MCP contract: an agent lists open markers assigned to it with the items under each, may set review, and is
/// refused `resolved` with `marker.resolveByAgent` while the marker stays unchanged.
#[test]
fn mcp_agent_may_review_but_never_resolve() {
    let (d, lib, _) = setup();
    let m = ok(&ove(&lib, &["markers", "add", "Demo", "--at", "5", "--assignee", "agent", "--note", "Tighten"]));
    let id = m["marker"]["id"].as_str().unwrap().to_string();
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
    let tools = request("tools/list", json!({}));
    let update = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "update_marker").unwrap();
    assert!(update["description"].as_str().unwrap().contains("Never set `resolved`"), "the rule is in the description");

    let call = |request: &mut dyn FnMut(&str, Value) -> Value, name: &str, args: Value| {
        request("tools/call", json!({"name": name, "arguments": args}))["result"].clone()
    };
    let listed = call(&mut request, "list_markers", json!({"project": "Demo", "status": "open", "assignee": "agent"}));
    assert_eq!(listed["isError"], false, "{listed}");
    assert_eq!(listed["structuredContent"]["markers"][0]["id"], id.as_str());
    assert_eq!(listed["structuredContent"]["markers"][0]["items"], json!(["i_a"]));

    let refused = call(&mut request, "update_marker", json!({"project": "Demo", "id": id, "status": "resolved"}));
    assert_eq!(refused["isError"], true);
    assert_eq!(refused["structuredContent"]["error"]["code"], "marker.resolveByAgent");
    let refused_add = call(&mut request, "add_marker", json!({"project": "Demo", "at": 1, "status": "resolved"}));
    assert_eq!(refused_add["structuredContent"]["error"]["code"], "marker.resolveByAgent");

    let reviewed = call(&mut request, "update_marker", json!({"project": "Demo", "id": id, "status": "review"}));
    assert_eq!(reviewed["isError"], false, "{reviewed}");
    let _ = child.kill();
    let _ = child.wait();

    let after = ok(&ove(&lib, &["markers", "list", "Demo"]));
    assert_eq!(after["markers"].as_array().unwrap().len(), 1, "the refused add changed nothing");
    assert_eq!(after["markers"][0]["status"], "review");
    assert_eq!(after["markers"][0]["note"], "Tighten");
}
