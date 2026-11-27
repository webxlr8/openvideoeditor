//! Generation over `ove mcp` (unify-agent-tool-layer group 6), against a local stand-in for the fal.ai queue: a
//! client without confirmation prompts cannot spend; a client with them asks the person (MCP elicitation), and only
//! an accept spends. The approved job runs in a background worker that fills the placeholder; the agent's undo
//! reverts placeholder and fill. No real keys; nothing leaves the machine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ove_core::ops::Op;
use ove_core::revision::Source;
use serde_json::{Value, json};

/// The fal queue stand-in: each submission completes at once with a small image. Returns its address and the bodies
/// of the submissions it received.
fn stand_in() -> (String, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let posts: Arc<Mutex<Vec<String>>> = Arc::default();
    let (b2, p2) = (base.clone(), posts.clone());
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let (base, posts) = (b2.clone(), p2.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':')
                        && k.trim().eq_ignore_ascii_case("content-length")
                    {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let (ctype, out): (&str, Vec<u8>) = match (method.as_str(), path.as_str()) {
                    ("POST", p) if p.starts_with("/fal-ai/") => {
                        posts.lock().unwrap().push(String::from_utf8_lossy(&body).to_string());
                        let v = json!({"request_id": "r1", "status_url": format!("{base}/fal-ai/x/requests/r1/status"),
                            "response_url": format!("{base}/fal-ai/x/requests/r1")});
                        ("application/json", v.to_string().into_bytes())
                    }
                    ("GET", p) if p.ends_with("/status") => ("application/json", br#"{"status":"COMPLETED"}"#.to_vec()),
                    ("GET", "/out.png") => ("image/png", b"\x89PNG\r\n\x1a\n-stand-in".to_vec()),
                    ("GET", _) => {
                        let v = json!({"images": [{"url": format!("{base}/out.png"), "width": 64, "height": 36, "content_type": "image/png"}]});
                        ("application/json", v.to_string().into_bytes())
                    }
                    _ => ("application/json", b"{}".to_vec()),
                };
                let mut w = c;
                let _ = write!(
                    w,
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    out.len()
                );
                let _ = w.write_all(&out);
            });
        }
    });
    (base, posts)
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_overlay", "name": "Overlay", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "t_overlay", "type": "text", "startTime": 0, "endTime": 1, "name": "Title", "textContent": "x"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p.close().unwrap();
    let config = d.path().join("config");
    (d, lib, config)
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(lib: &Path, config: &Path, fal: &str, elicitation: bool) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
            .arg("--library")
            .arg(lib)
            .args(["mcp", "--project", "Demo"])
            .env_remove("OVE_LIBRARY")
            .env("OVE_CONFIG_DIR", config)
            .env("OVE_FAL_QUEUE_URL", fal)
            .env("OVE_GENERATE_POLL_MS", "10")
            .env("FAL_KEY", "fal-stand-in-key-0123456789")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp { child, out, id: 0 };
        let caps = if elicitation { json!({"elicitation": {}}) } else { json!({}) };
        let init = m.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": caps, "clientInfo": {"name": "stand-in agent", "version": "1"}}),
        );
        assert!(init["result"].is_object(), "{init}");
        m
    }

    fn send(&mut self, v: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}));
        self.read()
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

fn bundle(lib: &Path) -> PathBuf {
    ove_store::Library::open(lib, Arc::new(ove_core::SystemClock)).unwrap().find("Demo").unwrap()
}

fn project(lib: &Path) -> ove_store::Project {
    ove_store::Project::open(&bundle(lib), ove_store::Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap()
}

fn error_code(r: &Value) -> &str {
    r["structuredContent"]["error"]["code"].as_str().unwrap_or_else(|| panic!("not an error: {r}"))
}

#[test]
fn a_client_without_confirmation_prompts_cannot_spend() {
    let (fal, posts) = stand_in();
    let (_d, lib, config) = setup();
    let mut m = Mcp::start(&lib, &config, &fal, false);
    let rev = project(&lib).rev();
    let req = json!({"prompt": "a lighthouse"});
    let q = m.call("generate_image", req.clone());
    assert_eq!(q["isError"], false, "{q}");
    let quote = q["structuredContent"]["quoteId"].as_str().unwrap().to_string();
    let r = m.call("generate_image", json!({"prompt": "a lighthouse", "confirm": quote}));
    assert_eq!(error_code(&r), "confirm.required");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("elicitation"), "{r}");
    // The app's confirmation is not a tool.
    let r = m.request("tools/call", json!({"name": "confirm_spend", "arguments": {"quoteId": quote}}));
    assert_eq!(r["error"]["data"]["code"], "tool.unknown");
    assert!(posts.lock().unwrap().is_empty());
    assert_eq!(project(&lib).rev(), rev);
    assert!(ove_generate::job::list(&bundle(&lib)).is_empty());
}

#[test]
fn the_person_approves_in_the_client_and_the_placeholder_fills() {
    let (fal, posts) = stand_in();
    let (_d, lib, config) = setup();
    let mut m = Mcp::start(&lib, &config, &fal, true);
    let req = json!({"prompt": "a lighthouse at dusk"});
    let quote = m.call("generate_image", req.clone())["structuredContent"]["quoteId"].as_str().unwrap().to_string();

    // A declined prompt spends nothing.
    m.id += 1;
    let call_id = m.id;
    let declined = json!({"prompt": "a lighthouse at dusk", "confirm": quote});
    m.send(&json!({"jsonrpc": "2.0", "id": call_id, "method": "tools/call", "params": {"name": "generate_image", "arguments": declined}}));
    let ask = m.read();
    assert_eq!(ask["method"], "elicitation/create", "{ask}");
    m.send(&json!({"jsonrpc": "2.0", "id": ask["id"], "result": {"action": "decline"}}));
    let r = m.read();
    assert_eq!(r["id"], call_id);
    assert_eq!(error_code(&r["result"]), "confirm.declined");
    assert!(posts.lock().unwrap().is_empty());

    // Approved: the person sees the price and the prompt.
    let quote = m.call("generate_image", req.clone())["structuredContent"]["quoteId"].as_str().unwrap().to_string();
    m.id += 1;
    let call_id = m.id;
    let args = json!({"prompt": "a lighthouse at dusk", "confirm": quote});
    m.send(&json!({"jsonrpc": "2.0", "id": call_id, "method": "tools/call", "params": {"name": "generate_image", "arguments": args}}));
    let ask = m.read();
    let message = ask["params"]["message"].as_str().unwrap();
    assert!(
        message.contains("$0.025") && message.contains("a lighthouse at dusk") && message.contains("Demo"),
        "{message}"
    );
    assert_eq!(ask["params"]["requestedSchema"]["required"][0], "approve");
    // A ping while the person decides is still answered.
    m.send(&json!({"jsonrpc": "2.0", "id": "p1", "method": "ping"}));
    assert_eq!(m.read()["id"], "p1");
    m.send(&json!({"jsonrpc": "2.0", "id": ask["id"], "result": {"action": "accept", "content": {"approve": true}}}));
    let r = m.read();
    assert_eq!(r["id"], call_id);
    let s = &r["result"]["structuredContent"];
    assert_eq!(r["result"]["isError"], false, "{r}");
    let item = s["itemId"].as_str().unwrap().to_string();
    assert!(s["line"].as_str().unwrap().contains(&item));
    let job = s["jobId"].as_str().unwrap().to_string();

    // The background worker finishes and fills the placeholder.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let st = m.call("generation_status", json!({"id": job, "wait": true}));
        let status = st["structuredContent"]["status"].as_str().unwrap_or_default().to_string();
        if status == "succeeded" {
            break;
        }
        assert!(!["failed", "cancelled"].contains(&status.as_str()), "{st}");
        assert!(Instant::now() < deadline, "{st}");
    }
    assert_eq!(posts.lock().unwrap().len(), 1, "one submission");
    let deadline = Instant::now() + Duration::from_secs(30);
    while project(&lib).doc().item(&item).is_some_and(|i| i.is_pending()) {
        assert!(Instant::now() < deadline, "the placeholder was not filled");
        std::thread::sleep(Duration::from_millis(100));
    }
    // The agent undoes its own changes: placeholder and fill go, the person's title stays.
    // The worker may still be closing the project: retry while it is locked.
    let mut u = m.call("undo_my_changes", json!({}));
    for _ in 0..50 {
        if u["structuredContent"]["error"]["code"] != "project.locked" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        u = m.call("undo_my_changes", json!({}));
    }
    assert_eq!(u["isError"], false, "{u}");
    let p = project(&lib);
    assert!(p.doc().item(&item).is_none());
    assert!(p.doc().item("a").is_some());
}
