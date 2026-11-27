//! Seeing and steering background work (add-background-tasks-and-media-derivatives, task group 6): the status bar's
//! Background Tasks popover (`list_tasks`, `cancel_task`, `pause_tasks`, `run_anyway` and the `tasksChanged` event,
//! sent at most four times a second), and the control-link method `tasks` that `ove tasks` and the agent tools
//! `list_tasks`, `wait_tasks` and `cancel_task` use.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ove_tasks::{PauseReason, Scheduler, TaskFilter, TaskInfo, TaskState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};

/// The popover redraws at most this often.
const EMIT_EVERY: Duration = Duration::from_millis(250);

/// One task as the popover shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub id: u32,
    pub kind: String,
    pub label: String,
    /// The project's bundle path, when the task belongs to one.
    pub project: Option<String>,
    /// export, interactive, derivative or analysis.
    pub priority: String,
    /// queued, running or paused.
    pub state: String,
    /// Why it is paused (`battery`, `playback`, …), when it is.
    pub pause_reason: Option<String>,
    pub progress: Option<f64>,
    /// Larger than the whole memory budget: it runs with nothing beside it.
    pub running_alone: bool,
}

/// An active reason background work is paused, with the words the popover shows after "Paused: ".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PauseView {
    pub reason: String,
    pub label: String,
}

/// Everything the popover shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TasksView {
    /// Unfinished tasks, running first, most urgent first.
    pub tasks: Vec<TaskView>,
    /// Active pause reasons (after "run anyway").
    pub paused: Vec<PauseView>,
    /// Reasons the person chose to run anyway.
    pub run_anyway: Vec<String>,
}

/// The background tasks changed (sent at most four times a second).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TasksChanged(pub TasksView);

fn reason_code(r: PauseReason) -> String {
    serde_json::to_value(r).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn parse_reason(s: &str) -> Option<PauseReason> {
    serde_json::from_value(Value::String(s.to_string())).ok()
}

fn state_name(s: TaskState) -> &'static str {
    match s {
        TaskState::Queued => "queued",
        TaskState::Running => "running",
        TaskState::Paused => "paused",
        TaskState::Done => "done",
        TaskState::Failed => "failed",
        TaskState::Cancelled => "cancelled",
    }
}

fn task_view(t: &TaskInfo) -> TaskView {
    TaskView {
        id: u32::try_from(t.id).unwrap_or(u32::MAX),
        kind: t.kind.clone(),
        label: t.label.clone(),
        project: t.project.clone(),
        priority: t.priority.as_str().to_string(),
        state: state_name(t.state).to_string(),
        pause_reason: t.pause_reason.map(reason_code),
        progress: t.progress,
        running_alone: t.running_alone,
    }
}

/// The popover's view of a scheduler.
pub fn view(s: &Scheduler) -> TasksView {
    TasksView {
        tasks: s.list(&TaskFilter::default()).iter().map(task_view).collect(),
        paused: s
            .pause_reasons()
            .into_iter()
            .map(|r| PauseView { reason: reason_code(r), label: r.label().into() })
            .collect(),
        run_anyway: s.overrides().into_iter().map(reason_code).collect(),
    }
}

/// Sends `tasksChanged` when tasks change, at most every 250 ms (the last change always gets out).
pub fn follow(app: &AppHandle, scheduler: &Scheduler) {
    let pending = Arc::new(AtomicBool::new(false));
    let (a, s) = (app.clone(), scheduler.clone());
    scheduler.subscribe(move |_| {
        if pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let (a, s, p) = (a.clone(), s.clone(), pending.clone());
        let spawned = std::thread::Builder::new().name("tasks-popover".into()).spawn(move || {
            std::thread::sleep(EMIT_EVERY);
            p.store(false, Ordering::SeqCst);
            let _ = TasksChanged(view(&s)).emit(&a);
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "could not send a task update");
        }
    });
}

#[tauri::command]
#[specta::specta]
pub fn list_tasks(app: AppHandle) -> AppResult<TasksView> {
    Ok(view(&app.state::<AppState>().tasks))
}

/// Cancels one task; false when it already finished.
#[tauri::command]
#[specta::specta]
pub fn cancel_task(app: AppHandle, id: u32) -> AppResult<bool> {
    Ok(app.state::<AppState>().tasks.cancel(u64::from(id)))
}

/// Pauses or resumes all background work (except export).
#[tauri::command]
#[specta::specta]
pub fn pause_tasks(app: AppHandle, paused: bool) -> AppResult<()> {
    app.state::<AppState>().tasks.set_pause(PauseReason::User, paused);
    Ok(())
}

/// "Run anyway" for one pause reason (`battery`, `lowPower`, `thermal`, `playback`, `export`), or undoes it.
#[tauri::command]
#[specta::specta]
pub fn run_anyway(app: AppHandle, reason: String, on: bool) -> AppResult<()> {
    let r = parse_reason(&reason)
        .filter(|r| *r != PauseReason::User)
        .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, format!("“{reason}” is not a pause reason.")))?;
    app.state::<AppState>().tasks.set_override(r, on);
    Ok(())
}

/// The control-link method `tasks` (see `ove_tasks::remote`).
pub fn tasks_method(s: &Scheduler, params: &Value) -> Result<Value, ove_control::Refusal> {
    ove_tasks::remote::handle(s, params).map_err(|r| ove_control::Refusal::new(r.code, r.message))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ove_tasks::{Class, Priority, TaskError, TaskSpec};

    use super::*;

    fn steps(n: u32) -> impl FnOnce(&ove_tasks::TaskCtx) -> Result<(), TaskError> + Send + 'static {
        move |ctx| {
            for i in 0..n {
                ctx.checkpoint()?;
                ctx.progress(f64::from(i) / f64::from(n));
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }
    }

    #[test]
    fn the_popover_view_shows_states_progress_and_pause_reasons() {
        let s = crate::tasks::testing::scheduler();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().display().to_string();
        let spec = |k: &str, p| TaskSpec::new(k, format!("{k} task"), p, Class::Cpu).project(project.clone());
        let a = s.submit(spec("skim", Priority::Derivative), steps(400));
        let b = s.submit(spec("captions", Priority::Analysis), steps(400));
        let start = Instant::now();
        while view(&s).tasks.iter().filter(|t| t.state == "running").count() < 2 {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        let v = view(&s);
        assert_eq!(v.tasks.len(), 2);
        assert!(v.tasks.iter().all(|t| t.project.as_deref() == Some(project.as_str())));
        assert_eq!(v.tasks.iter().find(|t| u64::from(t.id) == a).unwrap().priority, "derivative");
        // On battery: both pause with the reason; "run anyway" lifts it.
        s.set_pause(PauseReason::Battery, true);
        let start = Instant::now();
        loop {
            let v = view(&s);
            if v.tasks.iter().all(|t| t.state == "paused") {
                assert!(v.tasks.iter().all(|t| t.pause_reason.as_deref() == Some("battery")));
                assert_eq!(v.paused, vec![PauseView { reason: "battery".into(), label: "on battery".into() }]);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "{v:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
        s.set_override(PauseReason::Battery, true);
        assert!(view(&s).paused.is_empty());
        assert_eq!(view(&s).run_anyway, vec!["battery".to_string()]);
        // Cancel one: the other keeps going.
        assert!(s.cancel(b));
        s.wait_task(b, Duration::from_secs(2)).unwrap();
        assert_eq!(view(&s).tasks.len(), 1);
        s.cancel(a);
    }
}
