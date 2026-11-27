//! `ove generate` against a local stand-in for fal.ai (no keys, no spending).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-png-but-enough-for-the-test";

fn fal_stand_in() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let b2 = base.clone();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let base = b2.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let (ctype, out): (&str, Vec<u8>) = if target.starts_with("/fal-ai/flux/dev") {
                    ("application/json", serde_json::to_vec(&json!({"request_id": "r", "status_url": format!("{base}/q/r/status"), "response_url": format!("{base}/q/r"), "cancel_url": format!("{base}/q/r/cancel")})).unwrap())
                } else if target == "/q/r/status" {
                    ("application/json", br#"{"status":"COMPLETED"}"#.to_vec())
                } else if target == "/q/r" {
                    (
                        "application/json",
                        serde_json::to_vec(&json!({"images": [{"url": format!("{base}/out.png")}]})).unwrap(),
                    )
                } else {
                    ("image/png", PNG.to_vec())
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
    base
}

fn ove(lib: &Path, env: &[(&str, &str)], args: &[&str]) -> (Option<i32>, Value, Value) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ove"));
    c.arg("--library").arg(lib).arg("--json").args(args).env_remove("OVE_LIBRARY").env_remove("FAL_KEY");
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    let parse = |b: &[u8]| serde_json::from_slice(b).unwrap_or(Value::Null);
    (out.status.code(), parse(&out.stdout), parse(&out.stderr))
}

#[test]
fn plan_confirm_run_and_background() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let l = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = l.create("Promo").unwrap();
    p.apply(ove_store::library::starter_tracks(), ove_core::Source::Gui, None).unwrap();
    p.close().unwrap();
    let base = fal_stand_in();
    let env = [("OVE_FAL_QUEUE_URL", base.as_str()), ("OVE_GENERATE_POLL_MS", "10")];

    // Dry run: plan and estimate, nothing placed.
    let (code, v, _) = ove(&lib, &env, &["generate", "image", "Promo", "a lighthouse", "--dry-run"]);
    assert_eq!(code, Some(0));
    assert_eq!(v["plan"]["estimateUsd"], 0.025);
    assert_eq!(v["plan"]["trackId"], "t_overlay");
    // Without --yes: refused with the plan.
    let (code, _, e) = ove(&lib, &env, &["generate", "image", "Promo", "a lighthouse"]);
    assert_eq!(code, Some(1));
    assert_eq!(e["error"]["code"], "confirm.required");
    // Without a key (when the system credential store has none): refused before anything is placed.
    let (code, _, e) = ove(&lib, &env, &["generate", "image", "Promo", "a lighthouse", "--yes"]);
    if code == Some(1) {
        assert_eq!(e["error"]["code"], "key.missing");
    }

    // With a key: placeholder, run, output in the bundle, placeholder filled.
    let env =
        [("OVE_FAL_QUEUE_URL", base.as_str()), ("OVE_GENERATE_POLL_MS", "10"), ("FAL_KEY", "fal-test-key-0123456789")];
    let (code, v, e) = ove(&lib, &env, &["generate", "image", "Promo", "a lighthouse", "--yes", "--start", "1"]);
    assert_eq!(code, Some(0), "{e}");
    assert_eq!(v["status"], "succeeded");
    let bundle = lib.join("Promo.ove");
    assert_eq!(std::fs::read(bundle.join(v["output"].as_str().unwrap())).unwrap(), PNG);
    let (_, src, _) = ove(&lib, &[], &["source", "print", "Promo"]);
    let text = src["text"].as_str().unwrap();
    assert!(text.contains("<image") && text.contains("start={1}"), "{text}");
    assert!(!text.contains("pending={true}"), "{text}");
    let (_, hist, _) = ove(&lib, &[], &["history", "Promo"]);
    let summaries: Vec<&str> =
        hist["revisions"].as_array().unwrap().iter().map(|r| r["summary"].as_str().unwrap()).collect();
    assert!(summaries.iter().any(|s| s.starts_with("generate image: placeholder")));
    assert!(summaries.iter().any(|s| s.starts_with("generate image: a lighthouse")));

    // Background: returns at once; the worker finishes; status --wait sees it done.
    let (code, v, e) = ove(&lib, &env, &["generate", "image", "Promo", "harbour at night", "--yes", "--no-wait"]);
    assert_eq!(code, Some(0), "{e}");
    assert_eq!(v["started"], true);
    let id = v["id"].as_str().unwrap().to_string();
    let (_, s, _) = ove(&lib, &env, &["generate", "status", &id, "--wait"]);
    assert_eq!(s["status"], "succeeded", "{s}");
    let (_, list, _) = ove(&lib, &[], &["generate", "list", "Promo"]);
    assert_eq!(list["jobs"].as_array().unwrap().len(), 2);
    // Models list works without keys.
    let (code, m, _) = ove(&lib, &[], &["generate", "models", "--kind", "video"]);
    assert_eq!(code, Some(0));
    assert!(m["models"].as_array().unwrap().iter().all(|x| x["kind"] == "video"));
}

#[test]
fn jobs_whose_worker_died_are_failed_and_their_placeholder_removed() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let l = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = l.create("Promo").unwrap();
    p.apply(ove_store::library::starter_tracks(), ove_core::Source::Gui, None).unwrap();
    // A job left running by a worker that crashed: its placeholder is still on the timeline.
    let req = ove_generate::Request { prompt: "a lighthouse".into(), ..Default::default() };
    let plan = ove_generate::plan(p.doc(), ove_generate::Kind::Image, &req, "i_gen_dead").unwrap();
    p.apply(plan.ops.clone(), ove_core::Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let mut worker =
        Command::new(env!("CARGO_BIN_EXE_ove")).arg("--version").stdout(std::process::Stdio::null()).spawn().unwrap();
    let gone = worker.id();
    worker.wait().unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let mut job = ove_generate::new_job(&plan, &req, "g_dead_worker", now - 5 * 60_000);
    job.status = ove_generate::Status::Running;
    job.worker_pid = Some(gone);
    ove_generate::job::save(&bundle, &job).unwrap();

    let (code, list, e) = ove(&lib, &[], &["generate", "list", "Promo"]);
    assert_eq!(code, Some(0), "{e}");
    let j = &list["jobs"][0];
    assert_eq!(j["status"], "failed", "{list}");
    assert!(j["error"].as_str().unwrap().contains("stopped before finishing"));
    let (_, src, _) = ove(&lib, &[], &["source", "print", "Promo"]);
    assert!(!src["text"].as_str().unwrap().contains("pending={true}"), "placeholder removed: {src}");
}

/// fix-render-and-edit-correctness task 4.3, command-line path: the background worker is killed while the provider
/// still works on the request; `ove generate status` resumes the job with a new worker, which polls the saved request
/// (no second submission) and fills the placeholder.
#[test]
fn a_killed_worker_is_resumed_without_a_second_submission() {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let posts = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let polls = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let (base, posts, finished, polls) = (base.clone(), posts.clone(), finished.clone(), polls.clone());
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let (base, posts, finished, polls) = (base.clone(), posts.clone(), finished.clone(), polls.clone());
                std::thread::spawn(move || {
                    let mut r = BufReader::new(c.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let target = parts.next().unwrap_or("").to_string();
                    let mut len = 0;
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).unwrap();
                        if h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0; len];
                    r.read_exact(&mut body).unwrap();
                    let json_out = |v: Value| ("application/json", serde_json::to_vec(&v).unwrap());
                    let (ctype, out): (&str, Vec<u8>) = if method == "POST" {
                        posts.fetch_add(1, Ordering::SeqCst);
                        json_out(json!({"request_id": "rk", "status_url": format!("{base}/q/rk/status"),
                            "response_url": format!("{base}/q/rk"), "cancel_url": format!("{base}/q/rk/cancel")}))
                    } else if target == "/q/rk/status" {
                        polls.lock().unwrap().push(target.clone());
                        // The provider is still working until the test says it is done.
                        let s = if finished.load(Ordering::SeqCst) { "COMPLETED" } else { "IN_PROGRESS" };
                        json_out(json!({"status": s}))
                    } else if target == "/q/rk" {
                        json_out(json!({"images": [{"url": format!("{base}/out.png")}]}))
                    } else {
                        ("image/png", PNG.to_vec())
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
    }

    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let l = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = l.create("Promo").unwrap();
    p.apply(ove_store::library::starter_tracks(), ove_core::Source::Gui, None).unwrap();
    p.close().unwrap();
    let bundle = lib.join("Promo.ove");
    let env =
        [("OVE_FAL_QUEUE_URL", base.as_str()), ("OVE_GENERATE_POLL_MS", "20"), ("FAL_KEY", "fal-test-key-0123456789")];

    // Output to files, not pipes: on Windows the background worker inherits the starting process's handles, and a
    // pipe would stay open (so reading it would wait) until the worker ends.
    let out_file = d.path().join("start.json");
    let status = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(&lib)
        .args(["--json", "generate", "image", "Promo", "a lighthouse", "--yes", "--no-wait"])
        .env_remove("OVE_LIBRARY")
        .env_remove("FAL_KEY")
        .envs(env)
        .stdout(std::fs::File::create(&out_file).unwrap())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let v: Value = serde_json::from_slice(&std::fs::read(&out_file).unwrap()).unwrap();
    let id = v["id"].as_str().unwrap().to_string();

    // Wait until the worker has submitted the request, saved its handle and is polling.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let job = loop {
        let j = ove_generate::job::load(&bundle, &id).unwrap();
        if j.status == ove_generate::Status::Running && j.remote.is_some() && polls.lock().unwrap().len() >= 2 {
            break j;
        }
        assert!(std::time::Instant::now() < deadline, "the worker never started polling: {j:?}");
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let pid = job.worker_pid.expect("the worker records its process id");

    // Kill the worker outright, mid-job.
    let kill = if cfg!(windows) {
        Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output().unwrap()
    } else {
        Command::new("kill").args(["-9", &pid.to_string()]).output().unwrap()
    };
    assert!(
        kill.status.success(),
        "could not kill worker {pid}: {} {}",
        String::from_utf8_lossy(&kill.stdout),
        String::from_utf8_lossy(&kill.stderr)
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while ove_generate::job::process_alive(pid) {
        assert!(std::time::Instant::now() < deadline, "worker {pid} survived the kill");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let after_kill = ove_generate::job::load(&bundle, &id).unwrap();
    assert_eq!(after_kill.status, ove_generate::Status::Running, "the dead worker left the job running");
    // The app waits DEAD_WORKER_GRACE_MS without an update before it calls a worker dead; move the job's last
    // update back past that instead of waiting a minute.
    let mut aged = after_kill;
    aged.updated_at -= ove_generate::job::DEAD_WORKER_GRACE_MS + 1_000;
    ove_generate::job::save(&bundle, &aged).unwrap();

    // The provider finishes while no worker is running; the next status call resumes the job.
    finished.store(true, Ordering::SeqCst);
    let (code, s, e) = ove(&lib, &env, &["generate", "status", &id, "--wait"]);
    assert_eq!(code, Some(0), "{e}");
    assert_eq!(s["status"], "succeeded", "{s}");
    assert_eq!(posts.load(Ordering::SeqCst), 1, "resuming does not submit the request again");
    let done = ove_generate::job::load(&bundle, &id).unwrap();
    assert_ne!(done.worker_pid, Some(pid), "a new worker finished the job");
    assert_eq!(std::fs::read(bundle.join(done.output.as_deref().unwrap())).unwrap(), PNG);
    let (_, src, _) = ove(&lib, &[], &["source", "print", "Promo"]);
    let text = src["text"].as_str().unwrap();
    assert!(text.contains("<image"), "{text}");
    assert!(!text.contains("pending={true}"), "the placeholder is filled: {text}");
}
