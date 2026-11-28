//! Pinned MCP sessions (`ove mcp --project <p> [--turn <id>]`) and `use_project` (unify-agent-tool-layer task 4.1).
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

/// A library with two projects, P and Q, each with one title.
fn setup() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    for name in ["P", "Q"] {
        let mut p = library.create(name).unwrap();
        let ops: Vec<Op> = serde_json::from_value(json!([
            {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
            {"op": "addItem", "item": {"id": "a", "trackId": "t_main", "type": "text", "startTime": 0, "endTime": 1, "name": "Title", "textContent": name}}
        ]))
        .unwrap();
        p.apply(ops, Source::Gui, None).unwrap();
        p.close().unwrap();
    }
    (d, lib)
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(lib: &Path, extra: &[&str]) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
            .arg("--library")
            .arg(lib)
            .arg("mcp")
            .args(extra)
            .env_remove("OVE_LIBRARY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, out, id: 0 }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"))
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
    }

    /// Renames the title by a text edit on the session's project.
    fn rename(&mut self, to: &str) -> Value {
        let printed = self.call("print_timeline", json!({}));
        let text = printed["structuredContent"]["text"].as_str().unwrap_or_else(|| panic!("{printed}"));
        let line = text.lines().find(|l| l.contains("id=\"a\"")).unwrap().to_string();
        let at = line.find("name=\"").unwrap() + 6;
        let end = at + line[at..].find('"').unwrap();
        let renamed = format!("{}{to}{}", &line[..at], &line[end..]);
        let edits = json!([{"oldText": line, "newText": renamed}]);
        self.call("apply_timeline", json!({"rev": printed["structuredContent"]["rev"], "edits": edits}))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn history(lib: &Path, project: &str) -> Vec<Value> {
    let bundle = ove_store::Library::open(lib, Arc::new(ove_core::SystemClock)).unwrap().find(project).unwrap();
    let p = ove_store::Project::open(&bundle, ove_store::Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap();
    p.history().unwrap().into_iter().map(|h| serde_json::to_value(h).unwrap()).collect()
}

#[test]
fn two_pinned_sessions_each_edit_only_their_own_project() {
    let (_d, lib) = setup();
    let mut on_p = Mcp::start(&lib, &["--project", "P", "--turn", "turn-p"]);
    let mut on_q = Mcp::start(&lib, &["--project", "Q", "--turn", "turn-q"]);
    // The pinned surface does not offer the library tools.
    let names: Vec<String> = on_p.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(!names.contains(&"list_projects".to_string()) && names.contains(&"apply_timeline".to_string()));
    // Both sessions edit at the same time, each without naming its project.
    for round in 0..3 {
        assert_eq!(on_p.rename(&format!("P{round}"))["isError"], false);
        assert_eq!(on_q.rename(&format!("Q{round}"))["isError"], false);
    }
    // Naming the other project is refused, and that project is unchanged.
    let before = history(&lib, "Q").len();
    let r = on_p.call("apply_timeline", json!({"project": "Q", "source": "x"}));
    assert_eq!(r["structuredContent"]["error"]["code"], "project.pinned", "{r}");
    let r = on_p.call("use_project", json!({"project": "Q"}));
    assert_eq!(r["structuredContent"]["error"]["code"], "project.pinned", "{r}");
    assert_eq!(history(&lib, "Q").len(), before);
    // Naming its own project is fine.
    assert_eq!(on_p.call("show_project", json!({"project": "P"}))["isError"], false);
    // Each project's history shows only its own session, with the turn.
    for (project, turn) in [("P", "turn-p"), ("Q", "turn-q")] {
        let agent: Vec<Value> = history(&lib, project).into_iter().filter(|h| h["source"] == "mcp").collect();
        assert_eq!(agent.len(), 3, "{project}");
        let sessions: std::collections::BTreeSet<String> =
            agent.iter().map(|h| h["actor"]["session"].as_str().unwrap().to_string()).collect();
        assert_eq!(sessions.len(), 1, "{project}: {agent:?}");
        assert!(agent.iter().all(|h| h["actor"]["turn"] == turn), "{project}: {agent:?}");
    }
    let p_session =
        history(&lib, "P").into_iter().rev().find(|h| h["source"] == "mcp").unwrap()["actor"]["session"].clone();
    let q_session =
        history(&lib, "Q").into_iter().rev().find(|h| h["source"] == "mcp").unwrap()["actor"]["session"].clone();
    assert_ne!(p_session, q_session);
}

#[test]
fn use_project_sets_the_default_for_an_unbound_session() {
    let (_d, lib) = setup();
    let mut m = Mcp::start(&lib, &[]);
    assert_eq!(m.call("print_timeline", json!({}))["structuredContent"]["error"]["code"], "argument.missing");
    let r = m.call("use_project", json!({"project": "Q"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(m.rename("Renamed")["isError"], false);
    assert!(history(&lib, "Q").iter().any(|h| h["source"] == "mcp"));
    assert!(!history(&lib, "P").iter().any(|h| h["source"] == "mcp"));
    // An unbound session may still name another project, and switch.
    assert_eq!(m.call("show_project", json!({"project": "P"}))["isError"], false);
    assert_eq!(m.call("use_project", json!({"project": "P"}))["isError"], false);
}
