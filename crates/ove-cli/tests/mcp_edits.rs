//! `apply_timeline` over MCP: text edits and whole-file mode, refusals, and what text edits save on a 50-edit
//! session (unify-agent-tool-layer tasks 2.1 and 2.3).
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

/// A library with "Demo": `n` titles on Main.
fn setup(n: usize) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let mut ops: Vec<Op> = vec![
        serde_json::from_value(
            json!({"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}}),
        )
        .unwrap(),
    ];
    for i in 0..n {
        ops.push(
            serde_json::from_value(json!({"op": "addItem", "item": {"id": format!("i{i}"), "trackId": "t_main",
                "type": "text", "startTime": i, "endTime": i + 1, "textContent": format!("Title {i}"),
                "name": format!("Item {i}"), "fontSize": 64, "color": "#ffffff"}}))
            .unwrap(),
        );
    }
    p.apply(ops, Source::Gui, None).unwrap();
    p.close().unwrap();
    (d, lib)
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
    /// Bytes of the requests sent (what the agent spends on input).
    sent: usize,
}

impl Mcp {
    fn start(lib: &Path) -> Mcp {
        Mcp::start_with(lib, &[])
    }

    fn start_with(lib: &Path, env: &[(&str, &str)]) -> Mcp {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ove"));
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .arg("--library")
            .arg(lib)
            .arg("mcp")
            .env_remove("OVE_LIBRARY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, out, id: 0, sent: 0 }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": self.id, "method": "tools/call", "params": {"name": tool, "arguments": args}});
        let line = msg.to_string();
        self.sent += line.len() + 1;
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        let v: Value = serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"));
        v["result"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn element(text: &str, id: &str) -> String {
    let at = format!("id=\"{id}\"");
    text.lines().find(|l| l.contains(&at)).unwrap().to_string()
}

#[test]
fn text_edits_and_whole_file_both_apply_and_anchors_are_checked() {
    let (_d, lib) = setup(3);
    let mut m = Mcp::start(&lib);
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    let text = printed["structuredContent"]["text"].as_str().unwrap().to_string();
    let rev = printed["structuredContent"]["rev"].as_i64().unwrap();

    // Text edits: only the changed line goes over the wire.
    let line = element(&text, "i1");
    let r = m.call(
        "apply_timeline",
        json!({"project": "Demo", "rev": rev, "edits": [{"oldText": line, "newText": line.replace("Title 1", "Opening")}]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["changed"], true);
    assert_eq!(s["changedItems"][0]["id"], "i1");
    assert!(s["changedItems"][0]["text"].as_str().unwrap().contains("Opening"));
    // The text content for the model is the same receipt.
    let as_text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&as_text, s);

    // Whole-file mode still works.
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    let text = printed["structuredContent"]["text"].as_str().unwrap().replace("Title 2", "Closing");
    let r = m.call("apply_timeline", json!({"project": "Demo", "source": text}));
    assert_eq!(r["structuredContent"]["updated"], 1, "{r}");

    // An anchor that is not in the printed text: refused, naming the edit, nothing applied.
    let before = m.call("timeline_history", json!({"project": "Demo"}))["structuredContent"]["revisions"]
        .as_array()
        .unwrap()
        .len();
    let r = m.call(
        "apply_timeline",
        json!({"project": "Demo", "rev": rev, "edits": [{"oldText": "<text id=\"gone\"", "newText": ""}]}),
    );
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["error"]["code"], "edit.anchor");
    assert_eq!(r["structuredContent"]["error"]["details"]["index"], 0);
    let after = m.call("timeline_history", json!({"project": "Demo"}))["structuredContent"]["revisions"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(before, after);
    // Both modes at once, or neither: refused.
    let r = m.call("apply_timeline", json!({"project": "Demo", "edits": [], "source": "x"}));
    assert_eq!(r["structuredContent"]["error"]["code"], "argument.invalid");
}

/// A scripted 50-edit session: once with text edits anchored on the receipts' lines (printing once), once
/// resending the whole file every time (printing before each edit). Records the bytes sent.
#[test]
fn fifty_text_edits_send_far_fewer_bytes_than_whole_files() {
    const ITEMS: usize = 60;
    const EDITS: usize = 50;
    let (_d1, lib1) = setup(ITEMS);
    let mut m = Mcp::start(&lib1);
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    let text = printed["structuredContent"]["text"].as_str().unwrap().to_string();
    let mut rev = printed["structuredContent"]["rev"].as_i64().unwrap();
    let mut lines: Vec<String> = (0..ITEMS).map(|i| element(&text, &format!("i{i}"))).collect();
    for line in lines.iter_mut().take(EDITS) {
        let old = line.clone();
        let new = old.replace("Title", "Shot");
        let r = m.call(
            "apply_timeline",
            json!({"project": "Demo", "rev": rev, "edits": [{"oldText": old, "newText": new}]}),
        );
        assert_eq!(r["isError"], false, "{r}");
        rev = r["structuredContent"]["rev"].as_i64().unwrap();
        *line = r["structuredContent"]["changedItems"][0]["text"].as_str().unwrap().to_string();
    }
    let edit_bytes = m.sent;

    let (_d2, lib2) = setup(ITEMS);
    let mut w = Mcp::start(&lib2);
    for e in 0..EDITS {
        let printed = w.call("print_timeline", json!({"project": "Demo"}));
        let text = printed["structuredContent"]["text"].as_str().unwrap();
        let edited = text.replacen(&format!("\"Title {e}\""), &format!("\"Shot {e}\""), 1);
        let r = w.call("apply_timeline", json!({"project": "Demo", "source": edited}));
        assert_eq!(r["structuredContent"]["updated"], 1, "{r}");
    }
    let file_bytes = w.sent;
    let ratio = file_bytes as f64 / edit_bytes as f64;
    println!(
        "50 edits on {ITEMS} items: text edits {edit_bytes} bytes sent, whole files {file_bytes} bytes; {ratio:.1}x"
    );
    assert!(ratio >= 5.0, "text edits sent {edit_bytes} bytes, whole files {file_bytes}: only {ratio:.1}x");
}

impl Mcp {
    fn initialize(&mut self, client: &str) {
        self.id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": self.id, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": client, "version": "1"}}});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
    }

    /// Renames item `id` with a text edit and returns the `apply_timeline` result (the caller checks it).
    #[must_use]
    fn rename(&mut self, id: &str, to: &str) -> Value {
        let printed = self.call("print_timeline", json!({"project": "Demo"}));
        let text = printed["structuredContent"]["text"].as_str().unwrap();
        let line = element(text, id);
        let edits = json!([{"oldText": line, "newText": line.replacen("name=\"Item", &format!("name=\"{to}"), 1)}]);
        self.call(
            "apply_timeline",
            json!({"project": "Demo", "rev": printed["structuredContent"]["rev"], "edits": edits}),
        )
    }
}

/// Both sessions run with a short hold, so each lets go of the project soon after its call and the other's
/// writer (which waits up to `WAIT_FOR_SESSION` for it) gets it well inside that wait, even on a loaded machine.
#[test]
fn two_sessions_are_told_apart_and_each_undoes_only_its_own_change() {
    let (_d, lib) = setup(3);
    let short_hold = [("OVE_MCP_HOLD_MS", "200")];
    let mut one = Mcp::start_with(&lib, &short_hold);
    let mut two = Mcp::start_with(&lib, &short_hold);
    one.initialize("claude-code");
    two.initialize("codex");
    // On a failure, say which error and how long the call took: `project.locked` after about WAIT_FOR_SESSION is a
    // holder that did not let go; `project.locked` at once is a waiter that gave up early (see docs/flaky-tests.md).
    let failed = |who: &str, r: &Value, took: std::time::Duration| {
        format!("{who}'s rename failed with {} after {took:?}: {r}", r["structuredContent"]["error"]["code"])
    };
    let t0 = std::time::Instant::now();
    let r = one.rename("i0", "One");
    assert_eq!(r["isError"], false, "{}", failed("session one", &r, t0.elapsed()));
    let t0 = std::time::Instant::now();
    let r = two.rename("i1", "Two");
    assert_eq!(r["isError"], false, "{}", failed("session two", &r, t0.elapsed()));
    let h = one.call("timeline_history", json!({"project": "Demo"}));
    let revs = h["structuredContent"]["revisions"].as_array().unwrap();
    let last: Vec<&Value> = revs.iter().rev().take(2).map(|r| &r["actor"]).collect();
    assert_eq!(last[0]["client"], "codex", "{h}");
    assert_eq!(last[1]["client"], "claude-code");
    assert_eq!(last[0]["kind"], "mcp");
    assert_ne!(last[0]["session"], last[1]["session"]);
    // Session one undoes its own change, although session two's is the latest.
    let r = one.call("undo_timeline", json!({"project": "Demo"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = one.call("print_timeline", json!({"project": "Demo"}))["structuredContent"]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(element(&text, "i0").contains("name=\"Item 0\""), "{text}");
    assert!(element(&text, "i1").contains("name=\"Two 1\""), "{text}");
}

fn rename(m: &mut Mcp, id: &str, to: &str) -> Value {
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    let text = printed["structuredContent"]["text"].as_str().unwrap().to_string();
    let rev = printed["structuredContent"]["rev"].as_i64().unwrap();
    let line = element(&text, id);
    let new = line.replacen(&format!("Title {}", &id[1..]), to, 1);
    m.call("apply_timeline", json!({"project": "Demo", "rev": rev, "edits": [{"oldText": line, "newText": new}]}))
}

/// unify-agent-tool-layer task 4.3: the session keeps the project open between calls and keeps its lock; it lets go
/// once idle, so the app (here: this test opening the project for writing, as the app does) can open it. While the
/// app holds it, the session's edit is refused with `project.locked` (no app link here) and the session carries on.
#[test]
fn the_session_holds_the_project_between_calls_and_lets_the_app_open_it_when_idle() {
    let (_d, lib) = setup(3);
    let bundle = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap().find("Demo").unwrap();
    let mut m = Mcp::start_with(&lib, &[("OVE_MCP_HOLD_MS", "400")]);
    let r = rename(&mut m, "i0", "First");
    assert_eq!(r["isError"], false, "{r}");

    // Right after the call the session still holds the project, with the existing lock.
    let open =
        || ove_store::project::Project::open(&bundle, ove_store::project::Mode::Write, Arc::new(ove_core::SystemClock));
    let e = open().expect_err("the session holds the project");
    assert_eq!(e.code(), "project.locked");
    assert!(e.to_string().contains("ove"), "the holder is named: {e}");

    // Once the session is idle, the app opens it.
    let t0 = std::time::Instant::now();
    let app = loop {
        match open() {
            Ok(p) => break p,
            Err(e) if e.code() == "project.locked" && t0.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("the app could not open the project: {e}"),
        }
    };
    assert!(t0.elapsed() < std::time::Duration::from_secs(3), "released after {:?}", t0.elapsed());
    assert!(app.doc().to_string_pretty().contains("First"), "the session's edit is saved");

    // While the app holds it, the session's edit is refused and nothing is lost; the session goes on.
    let r = rename(&mut m, "i1", "Second");
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"]["code"], "project.locked");
    drop(app);
    let r = rename(&mut m, "i1", "Second");
    assert_eq!(r["isError"], false, "{r}");
    let history = m.call("timeline_history", json!({"project": "Demo"}));
    let summaries: Vec<String> = history["structuredContent"]["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["summary"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(summaries.iter().filter(|s| s.starts_with("source apply")).count(), 2, "{summaries:?}");
}

/// Deletes item `id` of Demo as the editor does: opens it for writing (waiting while an MCP session lets go of it),
/// applies the removal as a GUI revision and closes it.
fn delete_in_editor(lib: &Path, id: &str) {
    let bundle = ove_store::Library::open(lib, Arc::new(ove_core::SystemClock)).unwrap().find("Demo").unwrap();
    let t0 = std::time::Instant::now();
    let mut p = loop {
        match ove_store::project::Project::open(
            &bundle,
            ove_store::project::Mode::Write,
            Arc::new(ove_core::SystemClock),
        ) {
            Ok(p) => break p,
            Err(e) if e.code() == "project.locked" && t0.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("the editor could not open the project: {e}"),
        }
    };
    let del: Op = serde_json::from_value(json!({"op": "removeItems", "ids": [id]})).unwrap();
    p.apply(vec![del], Source::Gui, None).unwrap();
    p.close().unwrap();
}

/// The items of Demo now.
fn item_ids(lib: &Path) -> Vec<String> {
    let bundle = ove_store::Library::open(lib, Arc::new(ove_core::SystemClock)).unwrap().find("Demo").unwrap();
    let p =
        ove_store::project::Project::open(&bundle, ove_store::project::Mode::ReadOnly, Arc::new(ove_core::SystemClock))
            .unwrap();
    let mut ids: Vec<String> = p.doc().project.items.keys().cloned().collect();
    ids.sort();
    ids
}

/// An agent's edit to an item the person deleted meanwhile: without `prefer` it is refused with `edit.conflict`,
/// names the item and applies nothing; sent again with `prefer: "file"` (the person chose to bring it back) the
/// item is re-created with the edit.
#[test]
fn an_agent_edit_to_an_item_deleted_in_the_editor_is_refused_until_the_person_chooses_to_restore_it() {
    let (_d, lib) = setup(3);
    let mut m = Mcp::start_with(&lib, &[("OVE_MCP_HOLD_MS", "200")]);
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    let text = printed["structuredContent"]["text"].as_str().unwrap().to_string();
    let rev = printed["structuredContent"]["rev"].as_i64().unwrap();
    let line = element(&text, "i0");
    let edits = json!([{"oldText": line, "newText": line.replacen("Title 0", "Opening", 1)}]);
    delete_in_editor(&lib, "i0");

    let r = m.call("apply_timeline", json!({"project": "Demo", "rev": rev, "edits": edits}));
    assert_eq!(r["isError"], true, "{r}");
    let error = &r["structuredContent"]["error"];
    assert_eq!(error["code"], "edit.conflict", "{r}");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("\"Item 0\" (i0)"), "the clip is named by name and id: {message}");
    assert!(message.contains("Ask the person"), "{message}");
    assert!(message.contains("prefer: \"file\""), "{message}");
    let items = error["details"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{r}");
    assert_eq!(items[0]["id"], "i0");
    assert_eq!(items[0]["name"], "Item 0");
    assert_eq!(item_ids(&lib), ["i1", "i2"], "nothing was applied");

    let r = m.call("apply_timeline", json!({"project": "Demo", "rev": rev, "edits": edits, "prefer": "file"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["conflicts"][0]["id"], "i0", "the conflict is still reported: {r}");
    let text =
        m.call("print_timeline", json!({"project": "Demo"}))["structuredContent"]["text"].as_str().unwrap().to_string();
    assert!(element(&text, "i0").contains("Opening"), "re-created with the edit: {text}");
}

/// The dry run agrees with the apply: `check_timeline` without `prefer` reports an edit to an item deleted in the
/// editor as the same `edit.conflict` refusal; with `prefer: "file"` it passes. Neither call changes the project.
#[test]
fn check_timeline_reports_the_same_conflict_as_apply_for_an_item_deleted_in_the_editor() {
    let (_d, lib) = setup(3);
    let mut m = Mcp::start_with(&lib, &[("OVE_MCP_HOLD_MS", "200")]);
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    let text = printed["structuredContent"]["text"].as_str().unwrap().replacen("Title 0", "Opening", 1);
    delete_in_editor(&lib, "i0");

    let r = m.call("check_timeline", json!({"project": "Demo", "source": text}));
    assert_eq!(r["isError"], true, "{r}");
    let error = &r["structuredContent"]["error"];
    assert_eq!(error["code"], "edit.conflict", "{r}");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("\"Item 0\" (i0)"), "the clip is named by name and id: {message}");
    assert!(message.contains("Ask the person"), "{message}");
    assert_eq!(error["details"]["items"][0]["id"], "i0", "{r}");
    assert_eq!(error["details"]["items"][0]["name"], "Item 0", "{r}");

    let r = m.call("check_timeline", json!({"project": "Demo", "source": text, "prefer": "file"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["ok"], true, "{r}");
    assert_eq!(r["structuredContent"]["conflicts"][0]["id"], "i0", "the conflict is still listed: {r}");
    assert_eq!(item_ids(&lib), ["i1", "i2"], "a check changes nothing");
}

/// `ove source apply` keeps "file wins": with no `--prefer` an edit to an item deleted in the editor re-creates it.
#[test]
fn source_apply_still_brings_back_an_item_deleted_in_the_editor_by_default() {
    let (d, lib) = setup(3);
    let ove = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ove"))
            .arg("--library")
            .arg(&lib)
            .args(args)
            .env_remove("OVE_LIBRARY")
            .output()
            .unwrap()
    };
    let out = ove(&["source", "print", "Demo", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = serde_json::from_slice::<Value>(&out.stdout).unwrap()["text"]
        .as_str()
        .unwrap()
        .replacen("Title 0", "Opening", 1);
    let file = d.path().join("edited.tsx");
    std::fs::write(&file, text).unwrap();
    delete_in_editor(&lib, "i0");

    let out = ove(&["source", "apply", "Demo", file.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["conflicts"][0]["id"], "i0", "the conflict is reported: {v}");
    assert_eq!(item_ids(&lib), ["i0", "i1", "i2"], "the item is back");
}
