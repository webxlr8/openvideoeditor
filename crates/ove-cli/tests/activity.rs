//! Live agent activity over the control link (change add-agent-chat-context-and-activity, task 4.2): an MCP agent's
//! read reaches the running app as `notify`, and its edit, sent through the app, becomes a change the app shows —
//! both named after the agent's client.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ove_core::ops::Op;
use ove_core::revision::{Actor, Source};
use ove_store::project::{Mode, Project};
use ove_tools::activity;
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "main", "type": "text", "startTime": 0, "endTime": 2, "textContent": "One", "name": "First"}},
        {"op": "addItem", "item": {"id": "b", "trackId": "main", "type": "text", "startTime": 2, "endTime": 4, "textContent": "Hi", "name": "Title"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p.flush_projection();
    p.close().unwrap();
    (d, lib)
}

/// What the stand-in app showed: (kind, actor name, ids, range).
type Shown = Arc<Mutex<Vec<(String, String, Vec<String>, Option<(f64, f64)>)>>>;

/// A stand-in for the app: holds the project's write lock, applies edits sent to it, and shows activity the way the
/// desktop does — changes from the revisions the edit made, reads from `notify`.
fn fake_app(lib: &Path, bundle: &Path) -> (ove_control::Server, Shown) {
    let p = Arc::new(Mutex::new(Project::open(bundle, Mode::Write, Arc::new(ove_core::SystemClock)).unwrap()));
    let shown: Shown = Arc::default();
    let log = shown.clone();
    let server = ove_control::serve(
        lib,
        "test",
        Arc::new(move |method, params| {
            let mut p = p.lock().unwrap();
            match method {
                "context" => {
                    Ok(json!({"project": {"bundle": p.bundle(), "name": p.doc().project.name, "rev": p.rev()}}))
                }
                "apply" => {
                    let ops: Vec<Op> = serde_json::from_value(params["ops"].clone()).unwrap();
                    let source: Source = serde_json::from_value(params["source"].clone()).unwrap();
                    let actor: Option<Actor> = serde_json::from_value(params["actor"].clone()).unwrap();
                    let before = p.rev();
                    p.apply_as(ops, source, params["summary"].as_str().map(str::to_string), actor)
                        .map_err(|e| ove_control::Refusal::new(e.code(), e.to_string()))?;
                    for r in activity::since(p.bundle(), before, p.rev(), 50) {
                        let a = activity::of_ops(&r.ops);
                        let kind = if activity::by_agent(&r) { "change" } else { "person" };
                        let name = activity::actor_label(r.actor.as_ref(), r.source).1;
                        log.lock().unwrap().push((kind.into(), name, a.ids, None));
                    }
                    Ok(json!({"rev": p.rev()}))
                }
                "notify" => {
                    let n: activity::Notice = serde_json::from_value(params).unwrap();
                    assert!(ove_tools::bundle::same_bundle(&n.bundle, p.bundle()));
                    let name = activity::actor_label(n.actor.as_ref(), Source::Mcp).1;
                    let range = n.activity.start.zip(n.activity.end);
                    log.lock().unwrap().push((n.activity.kind, name, n.activity.ids, range));
                    Ok(json!({"shown": true}))
                }
                _ => Err(ove_control::Refusal::new("method.unknown", method)),
            }
        }),
    )
    .unwrap();
    (server, shown)
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
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
        Mcp { child, out, id: 0 }
    }

    fn send(&mut self, v: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}));
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn an_mcp_read_and_apply_produce_read_and_change_events_named_after_the_client() {
    let (_d, lib) = setup();
    let bundle = lib.join("Demo.ove");
    let (_server, shown) = fake_app(&lib, &bundle);
    let mut m = Mcp::start(&lib);
    m.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "1"}}),
    );
    m.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    // A full print: the whole timeline is read.
    let full = m.call("print_timeline", json!({"project": "Demo"}));
    let text = full["structuredContent"]["text"].as_str().unwrap().to_string();
    // A partial print: the items on those lines.
    let line = text.lines().position(|l| l.contains("id=\"b\"")).unwrap() + 1;
    m.call("print_timeline", json!({"project": "Demo", "from": line, "lines": 1}));
    // An edit, sent through the app.
    let rev = full["structuredContent"]["rev"].as_i64().unwrap();
    let old = text.lines().find(|l| l.contains("id=\"b\"")).unwrap().to_string();
    let edit = json!({"oldText": old, "newText": old.replace("textContent=\"Hi\"", "textContent=\"Hello\"")});
    let applied = m.call("apply_timeline", json!({"project": "Demo", "rev": rev, "edits": [edit]}));
    assert_eq!(applied["structuredContent"]["viaApp"], true, "{applied}");

    let t0 = Instant::now();
    while shown.lock().unwrap().len() < 3 && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let shown = shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 3, "{shown:?}");
    assert_eq!(shown[0].0, "read");
    assert_eq!(shown[0].1, "Claude Code");
    assert_eq!(shown[0].3, Some((0.0, 4.0)), "a full print reads the whole timeline");
    assert_eq!((shown[1].0.as_str(), shown[1].2.clone()), ("read", vec!["b".to_string()]));
    assert_eq!((shown[2].0.as_str(), shown[2].1.as_str()), ("change", "Claude Code"));
    assert_eq!(shown[2].2, ["b"]);
}
