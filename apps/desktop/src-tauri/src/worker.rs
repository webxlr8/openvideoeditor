//! Panic containment. A panic in a background thread, a command body or a command's future becomes an [`AppError`]
//! (and a log entry with backtrace, via the panic hook) instead of taking the app down or leaving the UI waiting.
//!
//! Every bridge command goes through one of these:
//! - the invoke handler is wrapped in [`contain_handler`], which catches a panic in any synchronous command (they run
//!   inside the handler, on the main thread) and rejects its promise with an `AppError`;
//! - asynchronous commands run their body through [`blocking`] (work on the blocking pool) or [`guarded`] (a future),
//!   because their futures run on the async runtime, outside the handler.

use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::thread::JoinHandle;

use crate::error::{AppError, AppResult, ErrorCode};

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

/// The error a command returns when its body panicked. The panic itself is in the log (panic hook).
pub fn command_failed(command: &str, detail: &str) -> AppError {
    tracing::error!(command, "command stopped by an internal error: {detail}");
    AppError::new(
        ErrorCode::WorkerPanic,
        "Something went wrong and this action was stopped. The app is still running; details are in the log.",
    )
}

/// Runs a command body, converting a panic into an `AppError`.
pub fn command<T>(name: &str, f: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => Err(command_failed(name, &panic_message(payload.as_ref()))),
    }
}

/// Runs a command body on the blocking thread pool (off the main thread), converting a panic into an `AppError`.
pub async fn blocking<T: Send + 'static>(
    name: &'static str,
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    match tauri::async_runtime::spawn_blocking(move || command(name, f)).await {
        Ok(r) => r,
        Err(e) => Err(command_failed(name, &e.to_string())),
    }
}

/// A command's future whose panics (while polled) become an `AppError`.
pub struct Guarded<F> {
    name: &'static str,
    fut: Pin<Box<F>>,
}

/// Wraps an async command body so a panic in it answers the UI with an `AppError`.
pub fn guarded<T, F: Future<Output = AppResult<T>>>(name: &'static str, fut: F) -> Guarded<F> {
    Guarded { name, fut: Box::pin(fut) }
}

impl<T, F: Future<Output = AppResult<T>>> Future for Guarded<F> {
    type Output = AppResult<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match catch_unwind(AssertUnwindSafe(|| this.fut.as_mut().poll(cx))) {
            Ok(p) => p,
            // The inner future is not polled again: this is its final answer.
            Err(payload) => Poll::Ready(Err(command_failed(this.name, &panic_message(payload.as_ref())))),
        }
    }
}

/// Wraps the bridge's invoke handler: a panic in a synchronous command is caught here (it would otherwise unwind
/// into the webview's callback and abort the app) and the command's promise is rejected with an `AppError`.
pub fn contain_handler<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let resolver = invoke.resolver.clone();
        let cmd = invoke.message.command().to_string();
        match catch_unwind(AssertUnwindSafe(|| handler(invoke))) {
            Ok(handled) => handled,
            Err(payload) => {
                let e = command_failed(&cmd, &panic_message(payload.as_ref()));
                // The command never answered (it panicked before its result was sent): answer it now so the UI does
                // not wait forever. Guarded in case it had answered after all.
                let _ = catch_unwind(AssertUnwindSafe(|| resolver.reject(e)));
                true
            }
        }
    }
}

/// Locks `m`, recovering it when a thread panicked while holding it: the poison is cleared and the value is used
/// as it was left. For plain state (snapshots, flags, queues) that is always consistent between statements.
pub fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!("recovering state that a failed task left locked");
            m.clear_poison();
            poisoned.into_inner()
        }
    }
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

    #[test]
    fn command_bodies_contain_panics() {
        let err = command("move_item", || -> AppResult<()> { panic!("injected") }).unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkerPanic);
        assert!(err.message.contains("still running"), "{}", err.message);
        assert_eq!(command("ok", || Ok(3)), Ok(3));

        let err = tauri::async_runtime::block_on(blocking("import_media", || -> AppResult<()> { panic!("injected") }))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkerPanic);
        assert_eq!(tauri::async_runtime::block_on(blocking("ok", || Ok(4))), Ok(4));

        let err = tauri::async_runtime::block_on(guarded("open_timeline", async {
            if true {
                panic!("injected");
            }
            Ok(())
        }))
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkerPanic);
        assert_eq!(tauri::async_runtime::block_on(guarded("ok", async { Ok(5) })), Ok(5));
    }

    #[test]
    fn poisoned_lock_is_recovered() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let mut g = relock(&m2);
            *g = 2;
            panic!("holder failed");
        })
        .join();
        assert!(m.is_poisoned());
        *relock(&m) += 1;
        assert!(!m.is_poisoned());
        assert_eq!(*m.lock().expect("poison cleared"), 3);
    }
}
