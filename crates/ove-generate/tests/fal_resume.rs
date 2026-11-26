//! A fal job whose worker died is resumed from the saved request: polling continues without a second submit, the
//! timeout still runs from the first submit, and a saved status URL off fal's domains is refused. One test, run
//! serially, because environment variables point the provider at the stand-ins.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_generate::job::RemoteRef;
use ove_generate::keys::{KeyStore, MemoryKeyStore};
use ove_generate::{Kind, Provider, Request, Status, execute, new_job, plan};
use serde_json::json;

type Seen = Arc<Mutex<Vec<(String, String, BTreeMap<String, String>)>>>;
type Route = dyn Fn(&str, &str) -> (u16, Vec<u8>) + Send + Sync;

/// A tiny HTTP server: `route(method, path) -> (status, body)`; requests are recorded.
fn serve(route: Arc<Route>) -> (String, Seen) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let (route, seen) = (route.clone(), s2.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let mut headers = BTreeMap::new();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                seen.lock().unwrap().push((method.clone(), path.clone(), headers));
                let (code, out) = route(&method, &path);
                let mut w = c;
                let _ = write!(
                    w,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    out.len()
                );
                let _ = w.write_all(&out);
            });
        }
    });
    (base, seen)
}

const KEY: &str = "fal-resume-test-key-0123456789";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-resumed-output";

fn doc() -> Document {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_overlay", "name": "Overlay", "order": 0, "type": "video"}}
    ]))
    .unwrap();
    apply_batch(&Document::new("p", "P", 1), &ops).unwrap().doc
}

fn now_ms() -> i64 {
    ove_core::Clock::now_ms(&ove_core::SystemClock)
}

#[test]
fn queued_fal_requests_resume_without_a_second_submit() {
    let host = Arc::new(Mutex::new(String::new()));
    let h2 = host.clone();
    let (base, seen) = serve(Arc::new(move |m, p| {
        let me = h2.lock().unwrap().clone();
        match (m, p) {
            ("POST", _) => (
                200,
                json!({
                    "request_id": "fresh",
                    "status_url": format!("{me}/fal-ai/x/requests/r9/status"),
                    "response_url": format!("{me}/fal-ai/x/requests/r9"),
                    "cancel_url": format!("{me}/fal-ai/x/requests/r9/cancel"),
                })
                .to_string()
                .into_bytes(),
            ),
            ("GET", "/fal-ai/x/requests/r9/status") => (200, br#"{"status":"COMPLETED"}"#.to_vec()),
            ("GET", "/fal-ai/x/requests/r9") => {
                (200, json!({"images": [{"url": format!("{me}/files/out.png")}], "seed": 3}).to_string().into_bytes())
            }
            ("GET", "/files/out.png") => (200, PNG.to_vec()),
            ("PUT", _) => (202, b"{}".to_vec()),
            _ => (404, b"{}".to_vec()),
        }
    }));
    *host.lock().unwrap() = base.clone();
    // SAFETY: the environment is only read through `std::env` in this test binary (std serialises those reads with
    // writes); no C code reads it concurrently, so changing it while other tests run cannot race.
    unsafe {
        std::env::set_var("OVE_FAL_QUEUE_URL", &base);
        std::env::set_var("OVE_GENERATE_POLL_MS", "10");
    }
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path();
    let keys = MemoryKeyStore::default();
    keys.set(Provider::Fal, KEY).unwrap();
    let req = Request { prompt: "a harbour at night".into(), model: Some("fal:fal-ai/x".into()), ..Default::default() };
    let p = plan(&doc(), Kind::Image, &req, "i_gen").unwrap();
    let posts = || seen.lock().unwrap().iter().filter(|s| s.0 == "POST").count();
    let remote = |status_url: String, submitted_at: i64| RemoteRef {
        status_url,
        response_url: format!("{base}/fal-ai/x/requests/r9"),
        cancel_url: Some(format!("{base}/fal-ai/x/requests/r9/cancel")),
        endpoint: "fal-ai/x".into(),
        request_id: Some("r9".into()),
        submitted_at,
    };

    // A fresh job: the queued request is saved on the job.
    let mut fresh = new_job(&p, &req, "g_fresh", now_ms());
    execute(bundle, &mut fresh, &keys, &|| false).unwrap();
    assert_eq!(posts(), 1);
    let saved = ove_generate::job::load(bundle, "g_fresh").unwrap();
    let r = saved.remote.expect("the remote request is saved");
    assert_eq!(r.status_url, format!("{base}/fal-ai/x/requests/r9/status"));
    assert_eq!(r.request_id.as_deref(), Some("fresh"));
    assert_eq!(saved.remote_id.as_deref(), Some("fresh"));

    // A job a dead worker left queued at fal: resumed, finished, no second submit.
    let mut job = new_job(&p, &req, "g_resume", now_ms());
    job.remote = Some(remote(format!("{base}/fal-ai/x/requests/r9/status"), now_ms() - 60_000));
    job.remote_id = Some("r9".into());
    ove_generate::job::save(bundle, &job).unwrap();
    execute(bundle, &mut job, &keys, &|| false).unwrap();
    assert_eq!(job.status, Status::Succeeded);
    assert_eq!(posts(), 1, "resuming does not submit again");
    assert_eq!(std::fs::read(bundle.join(job.output.as_deref().unwrap())).unwrap(), PNG);
    assert_eq!(job.details["requestId"], "r9");
    let status_poll = seen.lock().unwrap().iter().rev().find(|s| s.1.ends_with("/r9/status")).cloned().unwrap();
    assert_eq!(status_poll.2.get("authorization").map(String::as_str), Some(&*format!("Key {KEY}")));

    // Resuming past the timeout (measured from the first submit) gives up at once and cancels at fal.
    let polls_before = seen.lock().unwrap().iter().filter(|s| s.1.ends_with("/status")).count();
    let mut late = new_job(&p, &req, "g_late", now_ms());
    late.remote = Some(remote(format!("{base}/fal-ai/x/requests/r9/status"), now_ms() - 31 * 60_000));
    let e = execute(bundle, &mut late, &keys, &|| false).unwrap_err();
    assert_eq!(e.code, "provider.timeout");
    assert_eq!(late.status, Status::Failed);
    assert_eq!(seen.lock().unwrap().iter().filter(|s| s.1.ends_with("/status")).count(), polls_before);
    assert!(seen.lock().unwrap().iter().any(|s| s.0 == "PUT" && s.1 == "/fal-ai/x/requests/r9/cancel"));
    assert_eq!(posts(), 1);

    // A saved status URL on another host (a tampered job file) is refused; the key is not sent there.
    let (evil, evil_seen) = serve(Arc::new(|_, _| (200, br#"{"status":"COMPLETED"}"#.to_vec())));
    let mut forged = new_job(&p, &req, "g_forged", now_ms());
    forged.remote = Some(remote(format!("{evil}/status"), now_ms()));
    let e = execute(bundle, &mut forged, &keys, &|| false).unwrap_err();
    assert_eq!(e.code, "provider.failed");
    assert!(e.message.contains("status") && e.message.contains("not to fal.ai"), "{}", e.message);
    assert_eq!(forged.status, Status::Failed);
    assert!(evil_seen.lock().unwrap().is_empty(), "nothing reaches the other host");
    assert_eq!(posts(), 1);
}
