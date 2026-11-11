//! Background threads whose panics are contained: a panic becomes an [`AppError`] (and a log
//! entry with backtrace, via the panic hook) instead of taking the app down.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::JoinHandle;

use crate::error::{AppError, ErrorCode};

/// Extracts the text of a panic payload.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Runs `f` and converts a panic into an `AppError` naming the task.
pub fn contain<T>(task: &str, f: impl FnOnce() -> T) -> Result<T, AppError> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        let message = panic_message(payload.as_ref());
        tracing::error!(task, "background task failed: {message}");
        AppError::new(
            ErrorCode::WorkerPanic,
            format!("An internal error stopped {task}: {message}. Details are in the log."),
        )
    })
}

/// Spawns a named thread whose panic is contained (see [`contain`]).
pub fn spawn_contained<T: Send + 'static>(
    name: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<JoinHandle<Result<T, AppError>>> {
    let task = name.to_string();
    std::thread::Builder::new().name(name.to_string()).spawn(move || contain(&task, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_becomes_error_and_thread_joins() {
        let handle = spawn_contained("decode worker", || -> u32 { panic!("boom {}", 42) }).unwrap();
        let err = handle.join().expect("thread did not unwind past the boundary").unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkerPanic);
        assert!(err.message.contains("decode worker"));
        assert!(err.message.contains("boom 42"));
    }

    #[test]
    fn success_passes_through() {
        assert_eq!(spawn_contained("ok", || 7).unwrap().join().unwrap(), Ok(7));
    }

    #[test]
    fn str_payload() {
        let err = contain("t", || panic!("static")).unwrap_err();
        assert!(err.message.contains("static"));
    }
}
