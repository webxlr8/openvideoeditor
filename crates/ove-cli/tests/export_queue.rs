//! The export queue end to end (change extend-export-formats-and-queue, tasks 3.1–3.5): detached exports run by a
//! separate worker process in queue order, a failed job leaves no file, a job renders the revision it was queued at,
//! cancel stops a running job within 2 seconds, a killed worker's job restarts, and MCP `export_video`,
//! `job_status` and `cancel_job` see the same queue. Each test has its own queue folder. Needs `--features media`;
//! text-only projects, so no media fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]
#![cfg(feature = "media")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ove_render::queue::{ExportJob, JobStatus, Queue};
use serde_json::{Value, json};

struct Env {
    dir: tempfile::TempDir,
    lib: PathBuf,
    queue: PathBuf,
}

impl Env {
    /// A library with project Title: text items `t` [0, `secs`) and `u` [0, 1).
    fn new(secs: f64) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = library.create("Title").unwrap();
        let ops: Vec<ove_core::Op> = serde_json::from_value(json!([
            {"op": "addTrack", "track": {"id": "text", "name": "Text", "order": 0, "type": "video"}},
            {"op": "addTrack", "track": {"id": "more", "name": "More", "order": 1, "type": "video"}},
            {"op": "addItem", "item": {"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": secs, "textContent": "Long", "fontSize": 120}},
            {"op": "addItem", "item": {"id": "u", "trackId": "more", "type": "text", "startTime": 0, "endTime": 1, "textContent": "Short", "fontSize": 60}}
        ]))
        .unwrap();
        p.apply(ops, ove_core::Source::Gui, None).unwrap();
        p.close().unwrap();
        let queue = dir.path().join("queue");
        Env { dir, lib, queue }
    }

    fn ove(&self, args: &[&str]) -> Output {
        self.ove_env(args, &[])
    }

    fn ove_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ove"));
        c.current_dir(self.dir.path())
            .arg("--library")
            .arg(&self.lib)
            .arg("--json")
            .args(args)
            .env_remove("OVE_LIBRARY")
            .env("OVE_EXPORT_QUEUE_DIR", &self.queue);
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().unwrap()
    }

    /// The last stdout line as JSON, after checking the exit code.
    fn json(&self, args: &[&str]) -> Value {
        let out = self.ove(args);
        assert!(
            out.status.success(),
            "ove {args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(text.lines().last().unwrap()).unwrap()
    }

    fn q(&self) -> Queue {
        Queue::new(&self.queue)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Holds the worker lock as if a worker were running (this test process), so queued jobs wait.
    fn hold_worker(&self) -> PathBuf {
        std::fs::create_dir_all(&self.queue).unwrap();
        let lock = self.queue.join("worker.lock");
        std::fs::write(&lock, std::process::id().to_string()).unwrap();
        lock
    }

    fn wait_until(&self, id: &str, what: &str, timeout: Duration, ok: impl Fn(&ExportJob) -> bool) -> ExportJob {
        let start = Instant::now();
        loop {
            if let Some(j) = self.q().load(id)
                && ok(&j)
            {
                return j;
            }
            assert!(start.elapsed() < timeout, "{what}: {:?}", self.q().load(id));
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn no_partials(dir: &Path) {
    let left: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".partial"))
        .collect();
    assert!(left.is_empty(), "partial files left: {left:?}");
}

#[test]
fn detached_export_returns_at_once_and_status_wait_follows_it() {
    let e = Env::new(2.0);
    let t = Instant::now();
    let v = e.json(&["export", "Title", "out.mov", "--preset", "prores-422-hq-master", "--size", "360", "--detach"]);
    let returned_in = t.elapsed();
    let id = v["jobId"].as_str().unwrap().to_string();
    assert_eq!(v["kind"], "export");
    assert_eq!(v["requestedBy"], "cli");
    assert!(matches!(v["status"].as_str(), Some("queued" | "running")), "{v}");
    let done = e.json(&["export", "status", &id, "--wait"]);
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["result"]["videoFormat"], "ProRes 422 HQ");
    assert!(e.path("out.mov").is_file());
    no_partials(e.dir.path());
    let job = e.q().load(&id).unwrap();
    assert_eq!(job.attempt, 1);
    assert!(job.worker_pid.is_none());
    // The revision pin is released once the job has run.
    let bundle = job.project.clone();
    assert!(ove_store::pins::pinned(&bundle).is_empty());
    // The queue lists it, and a finished job cannot be cancelled: its file stays.
    let list = e.json(&["export", "queue"]);
    assert!(list["jobs"].as_array().unwrap().iter().any(|j| j["jobId"] == id.as_str()));
    let out = e.ove(&["export", "cancel", &id]);
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["code"], "job.finished");
    assert!(e.path("out.mov").is_file());
    eprintln!("RESULT detached export returned in {returned_in:?}");
}

#[test]
fn unknown_job_ids_are_refused_with_exit_code_1() {
    let e = Env::new(1.0);
    for args in [["export", "status", "x_nope"], ["export", "cancel", "x_nope"]] {
        let out = e.ove(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err: Value = serde_json::from_slice(&out.stderr).unwrap();
        assert_eq!(err["error"]["code"], "job.unknown");
        assert!(err["error"]["message"].as_str().unwrap().contains("x_nope"));
    }
    // A refused export creates no job.
    let out = e.ove(&["export", "Title", "bad.mov", "--range", "2-1", "--detach"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(e.q().list().is_empty(), "no job for a refused export");
}

#[test]
fn several_jobs_run_in_queue_order_and_a_failed_one_leaves_no_file() {
    let e = Env::new(1.0);
    let lock = e.hold_worker();
    let mut ids = Vec::new();
    for name in ["a.mov", "b.mov", "c.mov"] {
        let v = e.json(&["export", "Title", name, "--video-codec", "prores", "--size", "180", "--detach"]);
        ids.push(v["jobId"].as_str().unwrap().to_string());
    }
    // One that fails while encoding: an encoder that cannot be opened.
    let base = e.q().load(&ids[0]).unwrap();
    let mut bad = base.settings.clone();
    bad.encoder = Some("no-such-encoder".into());
    let failing = e.q().enqueue(&base.project, "Title", base.rev, bad, &e.path("bad.mov"), "test").unwrap();
    // Nothing has started while the worker lock is held.
    assert!(e.q().list().iter().all(|j| j.status == JobStatus::Queued));
    std::fs::remove_file(lock).unwrap();
    // Listing the queue starts a worker for the waiting jobs.
    e.json(&["export", "queue"]);
    let last = e.wait_until(&failing.id, "the failing job", Duration::from_secs(120), |j| j.status.finished());
    assert_eq!(last.status, JobStatus::Failed);
    assert!(last.error.as_deref().unwrap().contains("no-such-encoder"), "{:?}", last.error);
    assert!(!e.path("bad.mov").exists());
    let jobs: Vec<ExportJob> = ids.iter().map(|id| e.q().load(id).unwrap()).collect();
    for w in jobs.windows(2) {
        assert_eq!(w[0].status, JobStatus::Done);
        assert!(w[1].started_at.unwrap() >= w[0].finished_at.unwrap(), "jobs run one at a time, in queue order");
    }
    for name in ["a.mov", "b.mov", "c.mov"] {
        assert!(e.path(name).is_file(), "{name}");
    }
    no_partials(e.dir.path());
}

#[test]
fn an_edit_after_queuing_does_not_change_the_export_and_a_missing_revision_fails() {
    let e = Env::new(2.0);
    let lock = e.hold_worker();
    let v = e.json(&["export", "Title", "pinned.mov", "--video-codec", "prores", "--size", "180", "--detach"]);
    let id = v["jobId"].as_str().unwrap().to_string();
    let job = e.q().load(&id).unwrap();
    assert_eq!(ove_store::pins::pinned(&job.project), [job.rev]);
    // Delete the long item: the timeline is now 1 s.
    let mut p =
        ove_store::Project::open(&job.project, ove_store::Mode::Write, Arc::new(ove_core::SystemClock)).unwrap();
    let del: ove_core::Op = serde_json::from_value(json!({"op": "removeItems", "ids": ["t"]})).unwrap();
    p.apply(vec![del], ove_core::Source::Gui, None).unwrap();
    assert_eq!(p.doc().duration(), 1_000_000);
    p.close().unwrap();
    // A job whose revision cannot be reconstructed.
    let gone = e.q().enqueue(&job.project, "Title", 1, job.settings.clone(), &e.path("gone.mov"), "test").unwrap();
    std::fs::remove_file(lock).unwrap();
    let done = e.json(&["export", "status", &id, "--wait"]);
    assert_eq!(done["status"], "done", "{done}");
    let secs = done["result"]["durationSeconds"].as_f64().unwrap();
    assert!((secs - 2.0).abs() < 0.05, "the export renders the queued revision (2 s), not the edited one: {secs}");
    let failed = e.wait_until(&gone.id, "the missing revision", Duration::from_secs(60), |j| j.status.finished());
    assert_eq!(failed.status, JobStatus::Failed);
    assert!(failed.error.as_deref().unwrap().contains("export again"), "{:?}", failed.error);
    assert!(!e.path("gone.mov").exists());
}

#[test]
fn cancelling_a_running_export_stops_it_within_2_seconds() {
    let e = Env::new(120.0);
    let v = e.json(&["export", "Title", "long.mov", "--preset", "prores-422-hq-master", "--detach"]);
    let id = v["jobId"].as_str().unwrap().to_string();
    e.wait_until(&id, "the job starts encoding", Duration::from_secs(120), |j| {
        j.status == JobStatus::Running && j.frame > 0
    });
    let t = Instant::now();
    let c = e.json(&["export", "cancel", &id]);
    let job = e.wait_until(&id, "the cancel", Duration::from_secs(10), |j| j.status.finished());
    let took = t.elapsed();
    assert_eq!(job.status, JobStatus::Cancelled, "{c}");
    assert!(took <= Duration::from_secs(2), "cancel took {took:?}");
    assert!(!e.path("long.mov").exists());
    // The worker removes the partial file as the export stops.
    let start = Instant::now();
    while e.path("long.partial.mov").exists() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    no_partials(e.dir.path());
    eprintln!("RESULT cancel of a running export: status cancelled after {took:?}");
}

fn kill(pid: u32) {
    let st = if cfg!(windows) {
        Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output().unwrap().status
    } else {
        Command::new("kill").args(["-9", &pid.to_string()]).output().unwrap().status
    };
    assert!(st.success(), "could not kill {pid}");
}

#[test]
fn a_killed_workers_job_restarts_from_the_beginning() {
    let e = Env::new(8.0);
    let v = e.json(&["export", "Title", "crash.mov", "--video-codec", "prores", "--size", "360", "--detach"]);
    let id = v["jobId"].as_str().unwrap().to_string();
    let running = e
        .wait_until(&id, "the job starts", Duration::from_secs(120), |j| j.status == JobStatus::Running && j.frame > 0);
    kill(running.worker_pid.unwrap());
    // The next command that looks at the queue recovers the job (no grace period in the test) and starts a worker.
    let out = e.ove_env(&["export", "status", &id, "--wait"], &[("OVE_EXPORT_DEAD_GRACE_MS", "0")]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let job = e.q().load(&id).unwrap();
    assert_eq!(job.status, JobStatus::Done, "{job:?}");
    assert_eq!(job.attempt, 2);
    assert!(job.warnings.iter().any(|w| w.contains("restarted")), "{:?}", job.warnings);
    assert!(e.path("crash.mov").is_file());
    no_partials(e.dir.path());
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(e: &Env, client: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
            .current_dir(e.dir.path())
            .arg("--library")
            .arg(&e.lib)
            .arg("mcp")
            .env_remove("OVE_LIBRARY")
            .env("OVE_EXPORT_QUEUE_DIR", &e.queue)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp { child, out, id: 0 };
        m.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": client, "version": "1"}}));
        m
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut reply = String::new();
            self.out.read_line(&mut reply).unwrap();
            let v: Value = serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"));
            if v["id"] == self.id {
                return v["result"].clone();
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args}))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn mcp_export_video_job_status_and_cancel_job() {
    let e = Env::new(60.0);
    let mut m = Mcp::start(&e, "claude-code");
    // Queue (default: return at once), with format, quality and range.
    let r = m.call(
        "export_video",
        json!({"project": "Title", "out": e.path("short.mp4"), "format": "mp4", "quality": "low", "range": [0, 1], "resolution": "720p"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let id = r["structuredContent"]["jobId"].as_str().unwrap().to_string();
    assert_eq!(r["structuredContent"]["requestedBy"], "claude-code");
    // The command line sees the same job.
    let listed = e.json(&["export", "queue"]);
    assert!(
        listed["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["jobId"] == id.as_str() && j["requestedBy"] == "claude-code")
    );
    let s = m.call("job_status", json!({"job_id": id, "wait": true}));
    assert_eq!(s["structuredContent"]["status"], "done", "{s}");
    assert_eq!(s["structuredContent"]["result"]["durationSeconds"], 1.0);
    assert_eq!(s["structuredContent"]["settings"]["range"], json!([0.0, 1.0]));
    // A second, long one is cancelled.
    let r = m
        .call("export_video", json!({"project": "Title", "out": e.path("long.mov"), "preset": "ProRes 422 HQ master"}));
    let long = r["structuredContent"]["jobId"].as_str().unwrap().to_string();
    e.wait_until(&long, "the long job runs", Duration::from_secs(120), |j| j.status == JobStatus::Running);
    let c = m.call("cancel_job", json!({"job_id": long}));
    assert_eq!(c["isError"], false, "{c}");
    assert_eq!(c["structuredContent"]["status"], "cancelled", "{c}");
    assert!(!e.path("long.mov").exists());
    // Unknown ids are errors that name the id.
    for tool in ["job_status", "cancel_job"] {
        let u = m.call(tool, json!({"job_id": "x_missing"}));
        assert_eq!(u["isError"], true);
        assert_eq!(u["structuredContent"]["error"]["code"], "job.unknown");
        assert!(u["content"][0]["text"].as_str().unwrap().contains("x_missing"));
    }
    // `wait` returns the export's own result.
    let w = m.call(
        "export_video",
        json!({"project": "Title", "out": e.path("wav.wav"), "preset": "Audio WAV 16-bit", "range": [0, 2], "wait": true}),
    );
    assert_eq!(w["isError"], false, "{w}");
    assert_eq!(w["structuredContent"]["container"], "wav");
    assert!(w["structuredContent"]["jobId"].is_string());
    // An impossible combination is refused naming the fields, with no job.
    let before = e.q().list().len();
    let bad = m.call("export_video", json!({"project": "Title", "out": e.path("x.mp4"), "videoCodec": "prores"}));
    assert_eq!(bad["isError"], true);
    assert_eq!(bad["structuredContent"]["error"]["code"], "export.invalid");
    assert_eq!(e.q().list().len(), before);
}
