//! The scheduler as other processes see it: the control-link method `tasks` that the app serves for `ove tasks`
//! and the agent tools `list_tasks`, `wait_tasks` and `cancel_task`.
//!
//! Request `{"action": "list" | "wait" | "cancel", "bundle"?, "kind"?, "id"?, "timeoutMs"?}`:
//!
//! - `list`: `{"tasks": [TaskInfo…], "pauseReasons": […]}`: unfinished tasks, running first.
//! - `wait`: waits up to `timeoutMs` (at most [`WAIT_SLICE`] per call) for the matching tasks:
//!   `{"done", "remaining", "tasks", "pauseReasons"}`, with the tasks still unfinished.
//! - `cancel`: `{"cancelled": true, "task": TaskInfo}`; refused with `task.notFound` for an unknown or finished id.
//!
//! `bundle` matches a task's project however the path is spelled (it is compared as a file path).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::model::TaskFilter;
use crate::scheduler::Scheduler;

/// The longest one `wait` call blocks; callers wait longer by calling again.
pub const WAIT_SLICE: Duration = Duration::from_secs(10);

/// How long `cancel` waits for the task to reach its cancelled state before answering.
const CANCEL_WAIT: Duration = Duration::from_secs(1);

/// A refused request: a stable code and a plain-language message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
}

fn refused(code: &'static str, message: impl Into<String>) -> Refused {
    Refused { code, message: message.into() }
}

fn canonical(p: &Path) -> PathBuf {
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // Trailing separators and `.` parts do not change the place.
    p.components().collect()
}

/// The scheduler's project key for a bundle path a caller named (it may be spelled differently).
fn project_key(s: &Scheduler, bundle: &str) -> String {
    let want = canonical(Path::new(bundle));
    let all = s.list(&TaskFilter::default()).into_iter().chain(s.finished(&TaskFilter::default()));
    for t in all {
        if let Some(p) = t.project
            && canonical(Path::new(&p)) == want
        {
            return p;
        }
    }
    bundle.to_string()
}

fn pause_reasons(s: &Scheduler) -> Value {
    json!(s.pause_reasons())
}

/// Answers one `tasks` request.
pub fn handle(s: &Scheduler, params: &Value) -> Result<Value, Refused> {
    let filter = TaskFilter {
        project: params.get("bundle").and_then(Value::as_str).filter(|b| !b.is_empty()).map(|b| project_key(s, b)),
        kind: params.get("kind").and_then(Value::as_str).filter(|k| !k.is_empty()).map(str::to_string),
    };
    match params.get("action").and_then(Value::as_str).unwrap_or("list") {
        "list" => Ok(json!({"tasks": s.list(&filter), "pauseReasons": pause_reasons(s)})),
        "wait" => {
            let ms = params.get("timeoutMs").and_then(Value::as_u64).unwrap_or(0);
            let w = s.wait(&filter, Duration::from_millis(ms).min(WAIT_SLICE));
            Ok(json!({"done": w.done, "remaining": w.remaining, "tasks": s.list(&filter),
                      "pauseReasons": pause_reasons(s)}))
        }
        "cancel" => {
            let id = params.get("id").and_then(Value::as_u64).ok_or_else(|| refused("argument.missing", "id"))?;
            let unfinished = s.get(id).is_some_and(|t| !t.state.finished());
            if !unfinished || !s.cancel(id) {
                return Err(refused("task.notFound", format!("there is no unfinished task {id}")));
            }
            // A queued task ends at once; a running one at its next checkpoint.
            let end = s.wait_task(id, CANCEL_WAIT).or_else(|| s.get(id));
            Ok(json!({"cancelled": true, "task": end}))
        }
        other => Err(refused("argument.invalid", format!("unknown tasks action {other}"))),
    }
}
