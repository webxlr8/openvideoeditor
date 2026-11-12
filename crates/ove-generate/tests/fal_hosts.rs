//! The fal key goes only to fal's own API hosts: status, result and cancel URLs taken from a reply are checked, and
//! output downloads never carry it. A queued request that never finishes times out. One test, run serially, because
//! environment variables point the provider at the stand-ins.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use ove_generate::providers::{Run, fal};
use ove_generate::{Kind, Request};
use serde_json::json;

type Seen = Arc<Mutex<Vec<(String, String, BTreeMap<String, String>)>>>;
type Route = dyn Fn(&str, &str) -> (u16, String) + Send + Sync;

/// A tiny HTTP server: `route(method, path) -> (status, JSON or bytes)`; requests are recorded.
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
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                    out.len()
                );
            });
        }
    });
    (base, seen)
}

const KEY: &str = "fal-host-check-key-0123456789";

#[test]
fn fal_key_stays_on_fal_hosts() {
    // The attacker: any request it receives is recorded; it serves a "file" for downloads.
    let (evil, evil_seen) = serve(Arc::new(|_, _| (200, r#"{"status":"COMPLETED"}"#.to_string())));
    // The queue stand-in; the prompt picks which URL in the submit reply is spoofed.
    let evil2 = evil.clone();
    let host = Arc::new(Mutex::new(String::new()));
    let h2 = host.clone();
    let (base, seen) = serve(Arc::new(move |m, p| {
        let me = h2.lock().unwrap().clone();
        let mut urls = json!({
            "request_id": "r1",
            "status_url": format!("{me}/fal-ai/x/requests/r1/status"),
            "response_url": format!("{me}/fal-ai/x/requests/r1"),
            "cancel_url": format!("{me}/fal-ai/x/requests/r1/cancel"),
        });
        match (m, p) {
            ("POST", "/fal-ai/x/spoof-status") => urls["status_url"] = json!(format!("{evil2}/status")),
            ("POST", "/fal-ai/x/spoof-response") => urls["response_url"] = json!(format!("{evil2}/result")),
            ("POST", "/fal-ai/x/spoof-cancel") => {
                urls["cancel_url"] = json!(format!("{evil2}/cancel"));
                urls["status_url"] = json!(format!("{me}/fal-ai/x/requests/q/status"));
            }
            ("POST", "/fal-ai/x/https-lookalike") => {
                urls["status_url"] = json!("https://queue.fal.run.evil.example/status");
            }
            ("POST", "/fal-ai/x/stuck") => urls["status_url"] = json!(format!("{me}/fal-ai/x/requests/q/status")),
            ("POST", _) => {}
            ("GET", "/fal-ai/x/requests/q/status") => return (200, r#"{"status":"IN_QUEUE"}"#.into()),
            ("GET", "/fal-ai/x/requests/r1/status") => return (200, r#"{"status":"COMPLETED"}"#.into()),
            ("GET", "/fal-ai/x/requests/r1") => {
                return (200, json!({"images": [{"url": format!("{evil2}/files/out.png")}]}).to_string());
            }
            ("PUT", _) => return (202, "{}".into()),
            _ => return (404, "{}".into()),
        }
        (200, urls.to_string())
    }));
    *host.lock().unwrap() = base.clone();
    unsafe {
        std::env::set_var("OVE_FAL_QUEUE_URL", &base);
        std::env::set_var("OVE_GENERATE_POLL_MS", "10");
    }
    let out_dir = tempfile::tempdir().unwrap();
    // Each case is its own endpoint (`fal:fal-ai/x/<case>` posts to /fal-ai/x/<case>).
    let case = |endpoint: &str, cancel_after: Option<usize>| {
        let model = ove_generate::catalog::find(Some(&format!("fal:fal-ai/x/{endpoint}")), Kind::Image).unwrap();
        let req = Request { prompt: "a lighthouse".into(), ..Default::default() };
        let polls = std::cell::Cell::new(0usize);
        let cancelled = || {
            polls.set(polls.get() + 1);
            cancel_after.is_some_and(|n| polls.get() > n)
        };
        let mut progress = |_: Option<f64>, _: Option<&str>| {};
        let mut run = Run { key: KEY, cancelled: &cancelled, progress: &mut progress, out_dir: out_dir.path() };
        fal::run(&model, &req, "16:9", &mut run)
    };
    let evil_got_key = || evil_seen.lock().unwrap().iter().any(|(_, _, h)| h.values().any(|v| v.contains(KEY)));

    // A status or result URL on another host: refused before anything is sent there.
    for (endpoint, what) in [("spoof-status", "status"), ("spoof-response", "result")] {
        let e = case(endpoint, None).expect_err("refused");
        assert_eq!(e.code, "provider.failed");
        assert!(e.message.contains(what) && e.message.contains("not to fal.ai"), "{}", e.message);
    }
    let e = case("https-lookalike", None).expect_err("refused");
    assert!(e.message.contains("queue.fal.run.evil.example"), "{}", e.message);
    assert!(evil_seen.lock().unwrap().is_empty(), "nothing reaches the other host");

    // A cancel URL on another host is not called (the key stays home); the run still stops.
    let e = case("spoof-cancel", Some(2)).expect_err("cancelled");
    assert_eq!(e.code, "cancelled");
    assert!(evil_seen.lock().unwrap().is_empty(), "the spoofed cancel URL was not called");

    // The output is downloaded from wherever it is, without the key.
    let out = case("ok", None).expect("completes");
    assert_eq!(out.data.to_vec().unwrap(), br#"{"status":"COMPLETED"}"#);
    assert!(evil_seen.lock().unwrap().iter().any(|(m, p, _)| m == "GET" && p == "/files/out.png"));
    assert!(!evil_got_key(), "the download carried no key");
    let submit = seen.lock().unwrap().iter().find(|s| s.0 == "POST").cloned().unwrap();
    assert_eq!(submit.2.get("authorization").map(String::as_str), Some(&*format!("Key {KEY}")));

    // A request that never leaves the queue times out and is cancelled at fal.
    unsafe { std::env::set_var("OVE_GENERATE_TIMEOUT_S", "0.2") };
    let started = std::time::Instant::now();
    let e = case("stuck", None).expect_err("times out");
    unsafe { std::env::remove_var("OVE_GENERATE_TIMEOUT_S") };
    assert_eq!(e.code, "provider.timeout");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(seen.lock().unwrap().iter().any(|(m, p, h)| m == "PUT"
        && p == "/fal-ai/x/requests/r1/cancel"
        && h.get("authorization").is_some_and(|a| a.contains(KEY))));
    assert!(!evil_got_key());
}

/// Serves `len` bytes once, with or without a Content-Length header.
fn raw_body_server(len: usize, with_length: bool) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/out.mp4", l.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        loop {
            let mut h = String::new();
            if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
        }
        let head = if with_length {
            format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
        } else {
            "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nConnection: close\r\n\r\n".to_string()
        };
        let _ = c.write_all(head.as_bytes());
        let _ = c.write_all(&vec![7u8; len]);
    });
    url
}

#[test]
fn downloads_stream_to_disk_with_a_size_cap() {
    use ove_generate::providers::{Data, download_to};
    let dir = tempfile::tempdir().unwrap();
    let entries = || std::fs::read_dir(dir.path()).unwrap().count();

    // Within the cap: a temporary file in the destination folder, saved by renaming.
    let r = ureq::get(&raw_body_server(10_000, true)).call().unwrap();
    let data = download_to(r, dir.path(), 20_000).unwrap();
    let Data::File(tmp) = &data else { panic!("a download is a file") };
    assert_eq!(tmp.path().parent(), Some(dir.path()));
    assert_eq!(data.len(), 10_000);
    let dest = dir.path().join("g_1.mp4");
    data.save(&dest).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap().len(), 10_000);
    assert_eq!(entries(), 1, "only the saved file is left");

    // Over the cap, announced by Content-Length or found while streaming: refused, nothing left behind.
    for with_length in [true, false] {
        let r = ureq::get(&raw_body_server(30_000, with_length)).call().unwrap();
        let e = download_to(r, dir.path(), 20_000).unwrap_err();
        assert_eq!(e.code, "provider.tooLarge", "{with_length}");
        assert_eq!(entries(), 1, "the partial download was removed ({with_length})");
    }
}

#[test]
fn key_allowed_hosts() {
    let real = fal::QUEUE_URL;
    assert!(fal::key_allowed("https://queue.fal.run/fal-ai/x/requests/1/status", real));
    assert!(fal::key_allowed("https://rest.alpha.fal.ai/storage", real));
    assert!(fal::key_allowed("https://fal.run/fal-ai/x", real));
    assert!(!fal::key_allowed("http://queue.fal.run/x", real), "plain http");
    assert!(!fal::key_allowed("https://queue.fal.run.evil.example/x", real));
    assert!(!fal::key_allowed("https://evilfal.run/x", real));
    assert!(!fal::key_allowed("https://evil.example/?u=https://queue.fal.run", real));
    assert!(!fal::key_allowed("https://queue.fal.run@evil.example/x", real));
    assert!(!fal::key_allowed("https://v3.fal.media/files/out.png", real), "the CDN gets no key");
    assert!(!fal::key_allowed("not a url", real));
    // With a development override the override's own origin is trusted, nothing else on that machine.
    let dev = "http://127.0.0.1:9000";
    assert!(fal::key_allowed("http://127.0.0.1:9000/fal-ai/x/requests/1", dev));
    assert!(!fal::key_allowed("http://127.0.0.1:9001/x", dev));
}
