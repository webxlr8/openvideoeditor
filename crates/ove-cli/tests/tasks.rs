//! add-background-tasks-and-media-derivatives 6.2: `ove tasks list|wait|cancel` and the MCP tools `list_tasks`,
//! `wait_tasks`, `cancel_task`, without the app (headless: no background work exists) and against a stand-in app
//! that serves the control-link method `tasks` from a real scheduler, including time-out results.
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
use std::sync::Arc;
use std::time::{Duration, Instant};

use ove_tasks::{Class, Limits, Options, Priority, Scheduler, TaskCtx, TaskError, TaskSpec};
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

fn json_out(o: &std::process::Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!("{e}: {} / {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    })
}

fn json_err(o: &std::process::Output) -> Value {
    serde_json::from_slice(&o.stderr).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stderr)))
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let p = library.create("Demo").unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn steps(ms: u64) -> impl FnOnce(&TaskCtx) -> Result<(), TaskError> + Send + 'static {
    move |ctx| {
        for i in 0..ms / 5 {
            ctx.checkpoint()?;
            ctx.progress(i as f64 * 5.0 / ms as f64);
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

/// A stand-in for the app: a scheduler served over the control link as the desktop serves it.
fn fake_app(lib: &Path) -> (ove_control::Server, Scheduler) {
    let mut limits = Limits::for_machine(8, Some(16 * 1024 * 1024 * 1024));
    limits.tick = Duration::from_millis(20);
    let s = Scheduler::new(Options { limits, background_threads: true });
    let served = s.clone();
    let server = ove_control::serve(
        lib,
        "test",
        Arc::new(move |method, params| match method {
            "tasks" => {
                ove_tasks::remote::handle(&served, &params).map_err(|r| ove_control::Refusal::new(r.code, r.message))
            }
            _ => Err(ove_control::Refusal::new("method.unknown", method)),
        }),
    )
    .unwrap();
    (server, s)
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
        let mut m = Mcp { child, out, id: 0 };
        m.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
        );
        m.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        m
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
fn without_the_app_there_is_no_background_work() {
    let (_d, lib, _) = setup();
    let listed = json_out(&ove(&lib, &["tasks", "list", "--json"]));
    assert_eq!((listed["tasks"].clone(), listed["app"].clone()), (json!([]), json!(false)), "{listed}");
    let text = ove(&lib, &["tasks", "list", "Demo"]);
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("not running"));
    let waited = json_out(&ove(&lib, &["tasks", "wait", "--json", "--timeout", "5"]));
    assert_eq!((waited["done"].as_bool(), waited["remaining"].as_u64()), (Some(true), Some(0)));
    let c = ove(&lib, &["tasks", "cancel", "3", "--json"]);
    assert_eq!(c.status.code(), Some(1));
    assert_eq!(json_err(&c)["error"]["code"], "task.notFound");

    let mut m = Mcp::start(&lib);
    let tools = m.request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    for n in ["list_tasks", "wait_tasks", "cancel_task"] {
        assert!(tools.iter().any(|t| t["name"] == n), "missing {n}");
    }
    let l = m.call("list_tasks", json!({}));
    assert_eq!((l["isError"].as_bool(), l["structuredContent"]["app"].as_bool()), (Some(false), Some(false)));
    let c = m.call("cancel_task", json!({"id": 1}));
    assert_eq!(c["structuredContent"]["error"]["code"], "task.notFound");
    let bad = m.call("wait_tasks", json!({"timeoutSeconds": 9999}));
    assert_eq!(bad["structuredContent"]["error"]["code"], "argument.invalid");
}

#[test]
fn the_cli_and_mcp_list_wait_and_cancel_the_apps_tasks() {
    let (_d, lib, bundle) = setup();
    let (_server, s) = fake_app(&lib);
    let key = bundle.display().to_string();
    let spec =
        |k: &str| TaskSpec::new(k, format!("{k} of clip.mov"), Priority::Derivative, Class::Cpu).project(key.clone());
    let quick = s.submit(spec("thumbnail"), steps(300));
    let proxy = s.submit(spec("proxy"), steps(60_000));
    let elsewhere =
        s.submit(TaskSpec::new("skim", "other", Priority::Analysis, Class::Io).project("other"), steps(60_000));

    // List: every task, or one project's (named by project name), with id, kind, state and progress.
    let all = json_out(&ove(&lib, &["tasks", "list", "--json"]));
    assert_eq!(all["app"], true);
    assert!(all["tasks"].as_array().unwrap().len() >= 2, "{all}");
    let mine = json_out(&ove(&lib, &["tasks", "list", "Demo", "--json"]));
    assert!(mine["tasks"].as_array().unwrap().iter().all(|t| t["project"] == key.as_str()), "{mine}");
    assert!(mine["tasks"].as_array().unwrap().iter().any(|t| t["id"] == proxy && t["kind"] == "proxy"));
    let text = String::from_utf8_lossy(&ove(&lib, &["tasks", "list", "Demo", "--kind", "proxy"]).stdout).to_string();
    assert!(text.contains(&format!("#{proxy} ")) && text.contains("proxy of clip.mov"), "{text}");

    // Wait: the thumbnail finishes; the proxy times out with what remains.
    let w = json_out(&ove(&lib, &["tasks", "wait", "Demo", "--kind", "thumbnail", "--timeout", "20", "--json"]));
    assert_eq!((w["done"].as_bool(), w["timedOut"].as_bool()), (Some(true), Some(false)), "{w}");
    assert!(s.get(quick).unwrap().state.finished());
    let started = Instant::now();
    let w = json_out(&ove(&lib, &["tasks", "wait", "Demo", "--kind", "proxy", "--timeout", "0.5", "--json"]));
    assert_eq!(
        (w["done"].as_bool(), w["timedOut"].as_bool(), w["remaining"].as_u64()),
        (Some(false), Some(true), Some(1))
    );
    assert!(started.elapsed() < Duration::from_secs(10));

    // MCP: the same contract; a wait longer than one control-link slice still answers at its time-out.
    let mut m = Mcp::start(&lib);
    let l = m.call("list_tasks", json!({"project": "Demo", "kind": "proxy"}));
    assert_eq!(l["structuredContent"]["tasks"][0]["id"], proxy, "{l}");
    let started = Instant::now();
    let w = m.call("wait_tasks", json!({"project": "Demo", "kind": "proxy", "timeoutSeconds": 0.3}));
    let w = &w["structuredContent"];
    assert_eq!(
        (w["done"].as_bool(), w["timedOut"].as_bool(), w["remaining"].as_u64()),
        (Some(false), Some(true), Some(1))
    );
    assert!(started.elapsed() >= Duration::from_millis(300));

    // Cancel by an agent: the task stops and the result says cancelled; cancelling again is refused.
    let c = m.call("cancel_task", json!({"id": proxy}));
    assert_eq!(c["isError"], false, "{c}");
    assert_eq!(c["structuredContent"]["task"]["state"], "cancelled");
    let again = m.call("cancel_task", json!({"id": proxy}));
    assert_eq!(again["structuredContent"]["error"]["code"], "task.notFound");
    let w = m.call("wait_tasks", json!({"project": "Demo", "timeoutSeconds": 5}));
    assert_eq!(w["structuredContent"]["done"], true, "{w}");

    // The CLI cancels too.
    let c = json_out(&ove(&lib, &["tasks", "cancel", &elsewhere.to_string(), "--json"]));
    assert_eq!(c["task"]["state"], "cancelled");
}
