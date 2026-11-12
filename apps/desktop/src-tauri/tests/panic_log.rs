//! A panic in a background worker is contained (structured error, process keeps running) and the
//! log file contains the panic message and a backtrace.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_desktop_lib::error::ErrorCode;
use ove_desktop_lib::{logging, worker};

#[test]
fn worker_panic_is_logged_with_backtrace() {
    let dir = std::env::temp_dir().join(format!("ove-panic-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let guard = logging::init(&dir);

    let handle = worker::spawn_contained("decode worker", || -> u8 { panic!("forced decode panic") }).unwrap();
    let err = handle.join().expect("panic must not escape the worker").unwrap_err();
    assert_eq!(err.code, ErrorCode::WorkerPanic);

    // The process is still healthy: another worker runs normally.
    assert_eq!(worker::spawn_contained("next", || 5).unwrap().join().unwrap(), Ok(5));

    drop(guard); // flushes the non-blocking writer
    let log: String =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok()).collect();
    assert!(log.contains("forced decode panic"), "log: {log}");
    assert!(log.contains("backtrace:"), "log has a backtrace section");
    assert!(log.contains("worker_panic_is_logged_with_backtrace") || log.contains("panic_log"), "backtrace has frames");
    assert!(log.contains("decode worker"), "thread / task name logged");
    std::fs::remove_dir_all(&dir).unwrap();
}
