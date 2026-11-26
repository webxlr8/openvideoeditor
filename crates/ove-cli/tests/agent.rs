//! MCP server and edits forwarded to the running app (change 8).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use serde_json::{Value, json};

fn ove(lib: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "b", "trackId": "main", "type": "text", "startTime": 2, "endTime": 4, "textContent": "Hi", "name": "Title"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p.flush_projection();
    p.close().unwrap();
    (d, lib)
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(lib: &Path) -> Mcp {
        Mcp::start_with(lib, &[])
    }

    fn start_with(lib: &Path, extra: &[&str]) -> Mcp {
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
        let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(v["id"], self.id, "{v}");
        v
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
fn mcp_lists_tools_and_edits_through_the_timeline_text() {
    let (_d, lib) = setup();
    let mut m = Mcp::start(&lib);
    let init = m.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    m.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let tools = m.request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in
        ["print_timeline", "apply_timeline", "add_media", "look", "app_context", "export_video", "measure_loudness"]
    {
        assert!(names.contains(&n), "missing {n}");
    }
    for t in &tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
    }

    // Read, edit, check, apply, undo.
    let printed = m.call("print_timeline", json!({"project": "Demo"}));
    assert_eq!(printed["isError"], false, "{printed}");
    let text = printed["structuredContent"]["text"].as_str().unwrap().to_string();
    let edited = text.replacen("name=\"Title\"", "name=\"Opening title\"", 1);
    let checked = m.call("check_timeline", json!({"project": "Demo", "source": edited}));
    assert_eq!(checked["structuredContent"]["updated"], 1, "{checked}");
    let applied = m.call("apply_timeline", json!({"project": "Demo", "source": edited}));
    assert_eq!(applied["structuredContent"]["changed"], true, "{applied}");
    let h = m.call("timeline_history", json!({"project": "Demo"}));
    let last = h["structuredContent"]["revisions"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["source"], "mcp", "edits are recorded as mcp: {last}");

    // Refusals: bad text, unknown argument, missing argument — isError with a code.
    let bad = m.call("apply_timeline", json!({"project": "Demo", "source": "export default 1 +"}));
    assert_eq!(bad["isError"], true);
    assert!(bad["structuredContent"]["error"]["code"].is_string());
    let unknown = m.call("show_project", json!({"project": "Demo", "nope": 1}));
    assert_eq!(unknown["structuredContent"]["error"]["code"], "argument.unknown");
    let missing = m.call("show_project", json!({}));
    assert_eq!(missing["structuredContent"]["error"]["code"], "argument.missing");
    // App tools without the app: a clear refusal.
    let ctx = m.call("app_context", json!({}));
    assert_eq!(ctx["structuredContent"]["error"]["code"], "app.notRunning");
    // Unknown tool and method are protocol errors.
    assert_eq!(m.request("tools/call", json!({"name": "nope", "arguments": {}}))["error"]["code"], -32602);
    assert_eq!(m.request("nope/nope", json!({}))["error"]["code"], -32601);
    // Docs.
    let docs = m.call("docs", json!({"topic": "agents"}));
    assert!(docs["structuredContent"]["text"].as_str().unwrap().contains("source print"));
}

/// A stand-in for the app: holds the project's write lock and serves the control link like the desktop does.
fn fake_app(lib: &Path, bundle: &Path, refuse_first: bool) -> (ove_control::Server, Arc<Mutex<Project>>) {
    let p = Arc::new(Mutex::new(Project::open(bundle, Mode::Write, Arc::new(ove_core::SystemClock)).unwrap()));
    let held = p.clone();
    let refused = Arc::new(Mutex::new(!refuse_first));
    let server = ove_control::serve(
        lib,
        "test",
        Arc::new(move |method, params| {
            let mut p = held.lock().unwrap();
            match method {
                "context" => {
                    Ok(json!({"project": {"bundle": p.bundle(), "name": p.doc().project.name, "rev": p.rev()}}))
                }
                "apply" => {
                    let mut done = refused.lock().unwrap();
                    if !*done {
                        *done = true;
                        return Err(ove_control::Refusal::new("rev.changed", "the project changed; recompute"));
                    }
                    if params["expectRev"].as_i64() != Some(p.rev()) {
                        return Err(ove_control::Refusal::new("rev.changed", "the project changed; recompute"));
                    }
                    let ops: Vec<Op> = serde_json::from_value(params["ops"].clone()).unwrap();
                    let source: Source = serde_json::from_value(params["source"].clone()).unwrap();
                    p.apply(ops, source, params["summary"].as_str().map(str::to_string))
                        .map_err(|e| ove_control::Refusal::new(e.code(), e.to_string()))?;
                    Ok(json!({"rev": p.rev()}))
                }
                "restore" => {
                    let c = p.restore_as(params["rev"].as_i64().unwrap(), Source::Undo).unwrap();
                    Ok(json!({"rev": p.rev(), "summary": c.map(|c| c.record.summary)}))
                }
                _ => Err(ove_control::Refusal::new("method.unknown", method)),
            }
        }),
    )
    .unwrap();
    (server, p)
}

#[test]
fn edits_go_through_the_app_when_it_holds_the_project() {
    let (d, lib) = setup();
    let bundle = lib.join("Demo.ove");
    let (_server, held) = fake_app(&lib, &bundle, true);
    let before = held.lock().unwrap().rev();

    let out = ove(&lib, &["source", "print", "Demo", "--json"]);
    let text = serde_json::from_slice::<Value>(&out.stdout).unwrap()["text"].as_str().unwrap().replacen(
        "textContent=\"Hi\"",
        "textContent=\"Hello\"",
        1,
    );
    let file = d.path().join("e.tsx");
    std::fs::write(&file, text).unwrap();
    // The first attempt is refused with rev.changed; the command recomputes and succeeds.
    let out = ove(&lib, &["source", "apply", "Demo", file.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["viaApp"], true);
    let p = held.lock().unwrap();
    assert!(p.rev() > before);
    assert_eq!(p.doc().item("b").unwrap().get("textContent"), Some(json!("Hello")));
    assert_eq!(p.history().unwrap().last().unwrap().source, Source::Cli);
    drop(p);

    // Rename and undo also go through the app.
    let out = ove(&lib, &["project", "rename", "Demo", "Demo 2"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("(in the app)"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(held.lock().unwrap().doc().project.name, "Demo 2");
    let out = ove(&lib, &["undo", bundle.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(held.lock().unwrap().doc().project.name, "Demo");

    // App context through the CLI.
    let out = ove(&lib, &["app", "context", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["project"]["name"], "Demo");
}

#[test]
fn locked_by_another_process_without_the_app_is_refused() {
    let (_d, lib) = setup();
    let bundle = lib.join("Demo.ove");
    let _held = Project::open(&bundle, Mode::Write, Arc::new(ove_core::SystemClock)).unwrap();
    let out = ove(&lib, &["project", "rename", "Demo", "X", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "project.locked");
    // App commands without the app exit 3.
    assert_eq!(ove(&lib, &["app", "play"]).status.code(), Some(3));
}

#[test]
fn skills_install_writes_the_skill() {
    let d = tempfile::tempdir().unwrap();
    let out =
        Command::new(env!("CARGO_BIN_EXE_ove")).args(["skills", "install", "--dir"]).arg(d.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(d.path().join("openvideoeditor/SKILL.md")).unwrap();
    assert!(text.starts_with("---\nname: openvideoeditor\n"));
    assert!(text.contains("ove source apply"));
}

/// The in-app chat's server (`ove mcp --project … --turn …`): one project, only the tools for seeing and editing it,
/// media by id or name within the project, and a turn's edits only through the app.
#[test]
fn a_pinned_session_sees_and_edits_one_project_only() {
    let (_d, lib) = setup();
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    library.create("Other").unwrap().close().unwrap();
    let bundle = lib.join("Demo.ove");
    let outside = lib.parent().unwrap().join("secret.mp4");
    std::fs::write(&outside, b"not for the agent").unwrap();

    let mut m = Mcp::start_with(&lib, &["--project", "Demo", "--turn", "t1"]);
    let init = m.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}}));
    assert!(init["result"]["instructions"].as_str().unwrap().contains("open in OpenVideoEditor"), "{init}");

    let tools = m.request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in
        ["print_timeline", "apply_timeline", "look", "grab_frame", "measure_loudness", "detect_silence", "transcript"]
    {
        assert!(names.contains(&n), "{n} is offered");
    }
    for n in [
        "list_projects",
        "create_project",
        "rename_project",
        "open_in_app",
        "add_media",
        "remove_media",
        "relink_media",
        "export_video",
        "export_timeline_file",
        "import_timeline_file",
        "screenshot",
    ] {
        assert!(!names.contains(&n), "{n} is not offered");
    }
    for t in &tools {
        assert!(t["inputSchema"]["properties"].get("project").is_none(), "no project argument: {t}");
    }

    // The pinned project without naming it; another project, by name or path, is refused.
    let printed = m.call("print_timeline", json!({}));
    assert_eq!(printed["isError"], false, "{printed}");
    assert!(printed["structuredContent"]["text"].as_str().unwrap().contains("Title"));
    let named = m.call("print_timeline", json!({"project": bundle.display().to_string()}));
    assert_eq!(named["isError"], false, "naming the pinned project is fine: {named}");
    for other in [json!("Other"), json!(lib.join("Other.ove").display().to_string())] {
        let r = m.call("print_timeline", json!({ "project": other }));
        assert_eq!(r["structuredContent"]["error"]["code"], "project.pinned", "{r}");
    }

    // Tools that are not offered are refused when called anyway.
    for (tool, args) in [
        ("list_projects", json!({})),
        ("add_media", json!({"paths": [outside.display().to_string()]})),
        ("export_video", json!({"output": "x.mp4"})),
    ] {
        let r = m.call(tool, args);
        assert_eq!(r["structuredContent"]["error"]["code"], "tool.unavailable", "{tool}: {r}");
    }

    // Media is named within the project; a file by its path is refused before anything reads it.
    let r = m.call("grab_frame", json!({"media": outside.display().to_string()}));
    assert_eq!(r["structuredContent"]["error"]["code"], "media.notInProject", "{r}");
    let r = m.call("measure_loudness", json!({"media": "../secret.mp4"}));
    assert_eq!(r["structuredContent"]["error"]["code"], "media.notInProject", "{r}");

    // The transcript comes from the app (it has the speech models).
    let r = m.call("transcript", json!({"media": "nothing"}));
    assert_eq!(r["isError"], true, "{r}");
    // A turn's edits go through the app; with the app not running nothing is written to the bundle.
    let text = printed["structuredContent"]["text"].as_str().unwrap().replacen("name=\"Title\"", "name=\"T2\"", 1);
    let r = m.call("apply_timeline", json!({ "source": text }));
    assert_eq!(r["isError"], true, "{r}");
    let p = Project::open(&bundle, Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap();
    assert_eq!(p.doc().item("b").unwrap().name(), "Title", "the bundle is unchanged");
}

#[test]
fn a_pinned_session_needs_a_project_that_exists() {
    let (_d, lib) = setup();
    let out = ove(&lib, &["mcp", "--project", "Nope"]);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let out = ove(&lib, &["mcp", "--turn", "t1"]);
    assert!(!out.status.success(), "--turn needs --project");
}

/// Pictures from a pinned session are at most 1280 pixels on the long side, whatever size was asked for.
#[cfg(feature = "media")]
#[test]
fn a_pinned_session_caps_pictures() {
    let (_d, lib) = setup();
    let mut m = Mcp::start_with(&lib, &["--project", "Demo"]);
    m.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}}));
    let r = m.call("look", json!({"at": "3", "width": 2560}));
    assert_eq!(r["isError"], false, "{r}");
    let img = r["content"].as_array().unwrap().iter().find(|c| c["type"] == "image").expect("an image").clone();
    let png = decode_base64(img["data"].as_str().unwrap());
    let i = ove_analyze::Image::from_png(&png).unwrap();
    assert_eq!(i.width.max(i.height), 1280, "{}x{}", i.width, i.height);
}

#[cfg(feature = "media")]
fn decode_base64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::new();
    for c in bytes.chunks(4) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, &b)| n | (val(b) as u32) << (18 - 6 * i));
        out.push((n >> 16) as u8);
        if c.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}
