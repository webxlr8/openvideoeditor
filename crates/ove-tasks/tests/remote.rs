//! The control-link method `tasks` (`ove_tasks::remote`): list, wait with a time-out, cancel by id.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::time::{Duration, Instant};

use ove_tasks::remote::handle;
use ove_tasks::{Class, Limits, Options, PauseReason, Priority, Scheduler, TaskCtx, TaskError, TaskSpec};
use serde_json::json;

fn scheduler() -> Scheduler {
    let mut limits = Limits::for_machine(8, Some(16 * 1024 * 1024 * 1024));
    limits.tick = Duration::from_millis(20);
    Scheduler::new(Options { limits, background_threads: true })
}

/// A task that runs `ms` milliseconds in 5 ms steps.
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

#[test]
fn lists_waits_with_a_time_out_and_cancels() {
    let s = scheduler();
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("P.ove");
    std::fs::create_dir_all(&bundle).unwrap();
    let key = bundle.display().to_string();
    let spec = |k: &str| TaskSpec::new(k, format!("{k} task"), Priority::Derivative, Class::Cpu).project(key.clone());
    let quick = s.submit(spec("thumbnail"), steps(40));
    let slow = s.submit(spec("proxy"), steps(10_000));
    let other =
        s.submit(TaskSpec::new("skim", "elsewhere", Priority::Derivative, Class::Cpu).project("other"), steps(10_000));

    // The bundle may be spelled another way (here with a trailing separator); other projects are left out.
    let spelled = format!("{key}{}", std::path::MAIN_SEPARATOR);
    let listed = handle(&s, &json!({"action": "list", "bundle": spelled})).unwrap();
    let tasks = listed["tasks"].as_array().unwrap();
    let finished_quick = usize::from(s.get(quick).unwrap().state.finished());
    assert_eq!(tasks.len() + finished_quick, 2, "{listed}");
    assert!(tasks.iter().all(|t| t["project"] == key.as_str()));
    let t = tasks.iter().find(|t| t["kind"] == "proxy").unwrap();
    assert_eq!((t["priority"].as_str(), t["class"].as_str()), (Some("derivative"), Some("cpu")));
    assert_eq!(handle(&s, &json!({})).unwrap()["tasks"].as_array().unwrap().len() + finished_quick, 3);

    // Waiting for the thumbnails finishes; waiting for the proxy times out with the remaining count.
    let w = handle(&s, &json!({"action": "wait", "bundle": key, "kind": "thumbnail", "timeoutMs": 5000})).unwrap();
    assert_eq!((w["done"].as_bool(), w["remaining"].as_u64()), (Some(true), Some(0)));
    let started = Instant::now();
    let w = handle(&s, &json!({"action": "wait", "bundle": key, "kind": "proxy", "timeoutMs": 100})).unwrap();
    assert_eq!((w["done"].as_bool(), w["remaining"].as_u64()), (Some(false), Some(1)));
    assert_eq!(w["tasks"][0]["kind"], "proxy");
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(100) && took < Duration::from_secs(2), "{took:?}");
    // A long time-out is cut to one slice per call.
    assert_eq!(ove_tasks::remote::WAIT_SLICE, Duration::from_secs(10));

    // Paused work is listed as paused with its reason.
    s.set_pause(PauseReason::Battery, true);
    let listed = handle(&s, &json!({"action": "list", "kind": "proxy"})).unwrap();
    assert_eq!(listed["pauseReasons"], json!(["battery"]));
    s.set_pause(PauseReason::Battery, false);

    // Cancel by id: reported cancelled; a second cancel and unknown ids are refused.
    let started = Instant::now();
    let c = handle(&s, &json!({"action": "cancel", "id": slow})).unwrap();
    assert_eq!((c["cancelled"].as_bool(), c["task"]["state"].as_str()), (Some(true), Some("cancelled")));
    assert!(started.elapsed() < Duration::from_millis(200), "{:?}", started.elapsed());
    assert_eq!(handle(&s, &json!({"action": "cancel", "id": slow})).unwrap_err().code, "task.notFound");
    assert_eq!(handle(&s, &json!({"action": "cancel", "id": 999_999})).unwrap_err().code, "task.notFound");
    assert_eq!(handle(&s, &json!({"action": "cancel"})).unwrap_err().code, "argument.missing");
    assert_eq!(handle(&s, &json!({"action": "nope"})).unwrap_err().code, "argument.invalid");
    s.cancel(other);
}
