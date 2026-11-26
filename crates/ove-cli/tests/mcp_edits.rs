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
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
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

    fn rename(&mut self, id: &str, to: &str) {
        let printed = self.call("print_timeline", json!({"project": "Demo"}));
        let text = printed["structuredContent"]["text"].as_str().unwrap();
        let line = element(text, id);
        let edits = json!([{"oldText": line, "newText": line.replacen("name=\"Item", &format!("name=\"{to}"), 1)}]);
        let r = self.call(
            "apply_timeline",
            json!({"project": "Demo", "rev": printed["structuredContent"]["rev"], "edits": edits}),
        );
        assert_eq!(r["isError"], false, "{r}");
    }
}

#[test]
fn two_sessions_are_told_apart_and_each_undoes_only_its_own_change() {
    let (_d, lib) = setup(3);
    let mut one = Mcp::start(&lib);
    let mut two = Mcp::start(&lib);
    one.initialize("claude-code");
    two.initialize("codex");
    one.rename("i0", "One");
    two.rename("i1", "Two");
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
