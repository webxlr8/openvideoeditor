//! Background tasks (add-background-tasks-and-media-derivatives, task 6.2): `list_tasks`, `wait_tasks`, `cancel_task`.
//!
//! Background work (thumbnails, skim sheets, proxies, captions, export, analysis) runs in the app's scheduler, so
//! these tools ask the running app over the control link (method `tasks`). Without the app no background work exists:
//! listing finds none, waiting is done at once, and there is nothing to cancel. The command line works the same
//! way (`ove tasks list|wait|cancel`), headless on bundles.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ctx::{Surface, ToolCtx};
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

/// Default and largest `wait_tasks` time-out, seconds.
pub const WAIT_DEFAULT_S: f64 = 30.0;
pub const WAIT_MAX_S: f64 = 600.0;
/// The longest one control-link call waits (the app caps it at 10 s); longer waits call again.
const SLICE_MS: u64 = 10_000;

const NO_APP: &str = "The app is not running: background tasks run only in the app, so none exist.";

/// The bundle a call is scoped to: the named project, or a pinned session's project; `None` means every project.
fn scope(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Option<String>, Fail> {
    let bundle = match a.project() {
        Some(p) => Some(ctx.bundle(Some(p))?),
        None if ctx.surface() == Surface::Pinned => Some(ctx.bundle(None)?),
        None => None,
    };
    Ok(bundle.map(|b| b.display().to_string()))
}

/// Calls the app's `tasks` method; `None` when the app is not running.
fn call(ctx: &mut dyn ToolCtx, params: Value) -> Result<Option<Value>, Fail> {
    match ctx.app("tasks", params) {
        Ok(v) => Ok(Some(v)),
        Err(f) if f.code == "app.notRunning" => Ok(None),
        Err(f) => Err(f),
    }
}

fn params(action: &str, bundle: &Option<String>, a: &Args) -> Value {
    let mut p = json!({"action": action});
    if let Some(b) = bundle {
        p["bundle"] = json!(b);
    }
    if let Some(k) = a.str("kind").filter(|k| !k.is_empty()) {
        p["kind"] = json!(k);
    }
    p
}

/// One line per task: `#12 running 45% · Proxy: interview.mov (proxy, derivative)`.
pub fn task_line(t: &Value) -> String {
    let state = t["state"].as_str().unwrap_or("?");
    let mut s = format!("#{} {state}", t["id"]);
    if let Some(r) = t.get("pauseReason").and_then(Value::as_str) {
        s.push_str(&format!(" ({r})"));
    }
    if let Some(p) = t.get("progress").and_then(Value::as_f64) {
        s.push_str(&format!(" {:.0}%", p * 100.0));
    }
    s.push_str(&format!(
        " · {} ({}, {})",
        t["label"].as_str().unwrap_or(""),
        t["kind"].as_str().unwrap_or(""),
        t["priority"].as_str().unwrap_or("")
    ));
    if t.get("runningAlone").and_then(Value::as_bool) == Some(true) {
        s.push_str(" · running alone");
    }
    s
}

fn lines(tasks: &[Value]) -> String {
    tasks.iter().map(task_line).collect::<Vec<_>>().join("\n")
}

fn paused_line(v: &Value) -> Option<String> {
    let r: Vec<&str> = v.get("pauseReasons")?.as_array()?.iter().filter_map(Value::as_str).collect();
    (!r.is_empty()).then(|| format!("paused: {}", r.join(", ")))
}

pub fn list_tasks(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = scope(ctx, a)?;
    let Some(mut v) = call(ctx, params("list", &bundle, a))? else {
        return Ok(Receipt::new(json!({"tasks": [], "pauseReasons": [], "app": false}), NO_APP));
    };
    v["app"] = json!(true);
    let tasks = v["tasks"].as_array().cloned().unwrap_or_default();
    let mut text = if tasks.is_empty() { "no background tasks".to_string() } else { lines(&tasks) };
    if let Some(p) = paused_line(&v) {
        text.push_str(&format!("\n{p}"));
    }
    Ok(Receipt::new(v, text))
}

pub fn wait_tasks(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let timeout = a.f64("timeoutSeconds").unwrap_or(WAIT_DEFAULT_S);
    if !timeout.is_finite() || !(0.0..=WAIT_MAX_S).contains(&timeout) {
        return Err(Fail::new("argument.invalid", format!("timeoutSeconds must be between 0 and {WAIT_MAX_S}")));
    }
    let bundle = scope(ctx, a)?;
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut p = params("wait", &bundle, a);
        p["timeoutMs"] = json!((left.as_millis() as u64).min(SLICE_MS));
        let Some(v) = call(ctx, p)? else {
            return Ok(Receipt::new(
                json!({"done": true, "remaining": 0, "timedOut": false, "tasks": [], "app": false}),
                NO_APP,
            ));
        };
        let done = v["done"].as_bool() == Some(true);
        if done || left.is_zero() {
            let remaining = v["remaining"].as_u64().unwrap_or(0);
            let tasks = v["tasks"].as_array().cloned().unwrap_or_default();
            let text = if done {
                "all matching tasks are done".to_string()
            } else {
                let mut t =
                    format!("timed out after {timeout} s with {remaining} task(s) unfinished\n{}", lines(&tasks));
                if let Some(p) = paused_line(&v) {
                    t.push_str(&format!("\n{p}"));
                }
                t
            };
            let out = json!({"done": done, "remaining": remaining, "timedOut": !done, "tasks": tasks,
                             "pauseReasons": v["pauseReasons"], "app": true});
            return Ok(Receipt::new(out, text));
        }
    }
}

pub fn cancel_task(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let id = a.i64("id").filter(|i| *i > 0).ok_or_else(|| Fail::new("argument.invalid", "id must be a task id"))?;
    let Some(v) = call(ctx, json!({"action": "cancel", "id": id}))? else {
        return Err(Fail::new("task.notFound", format!("{NO_APP} There is no task {id} to cancel.")));
    };
    let state = v.pointer("/task/state").and_then(Value::as_str).unwrap_or("cancelling").to_string();
    Ok(Receipt::new(v, format!("task {id} {state}")))
}
