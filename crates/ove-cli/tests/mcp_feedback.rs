//! `send_feedback` over `ove mcp` makes no network request (unify-agent-tool-layer task 7.1). The session runs with
//! every way out of the machine the app knows pointed at a local trap — the HTTP(S) proxy variables and each provider
//! and update address — and the trap must see no connection, while the report lands in the local queue.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

#[test]
fn send_feedback_stays_on_this_computer() {
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    let trap_url = format!("http://{}", trap.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = hits.clone();
    std::thread::spawn(move || {
        for _ in trap.incoming().flatten() {
            h2.fetch_add(1, Ordering::SeqCst);
        }
    });
    let d = tempfile::tempdir().unwrap();
    let (lib, config) = (d.path().join("lib"), d.path().join("config"));
    ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ove"));
    cmd.arg("--library").arg(&lib).arg("mcp").env_remove("OVE_LIBRARY").env("OVE_CONFIG_DIR", &config);
    for var in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "OVE_FAL_QUEUE_URL",
        "OVE_OPENAI_URL",
        "OVE_ELEVENLABS_URL",
        "OVE_ANTHROPIC_BASE_URL",
        "OVE_UPDATE_URL",
    ] {
        cmd.env(var, &trap_url);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut id = 0;
    let mut request = |method: &str, params: Value| -> Value {
        id += 1;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "stand-in agent"}}),
    );
    let r = request(
        "tools/call",
        json!({"name": "send_feedback", "arguments": {"category": "confusing", "severity": "low", "summary": "The receipt of apply_timeline does not say which track an item moved to."}}),
    );
    let s = &r["result"]["structuredContent"];
    assert_eq!((s["stored"].as_bool(), s["sent"].as_bool()), (Some(true), Some(false)), "{r}");
    let queued = ove_tools::tools::feedback::list(&config);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].client.as_deref(), Some("stand-in agent"));
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(hits.load(Ordering::SeqCst), 0, "no network connection was made");
}
