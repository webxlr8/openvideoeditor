//! The control link (roadmap change 8): the `ove` command line and MCP server reach the running app through a
//! loopback port published in the library folder (`ove-control`). The app answers what it shows (`context`), opens
//! projects, drives the transport, and applies edits to the open project — the CLI cannot take the write lock while
//! the app holds it, so it sends its ops here and they become revisions of the app's session.

use std::path::Path;
use std::sync::Mutex;

use ove_core::ops::Op;
use ove_core::revision::Source;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::AppState;
use crate::error::AppResult;
use crate::projects::{ProjectView, emit, lock};
use crate::transport::TransportCommand;
use crate::worker::relock;

/// What the UI shows that only it knows: the selection and the viewer mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    pub selection: Vec<String>,
    /// `timeline` or `source`.
    pub viewer: String,
}

#[derive(Default)]
pub struct Control {
    ui: Mutex<UiState>,
    server: Mutex<Option<ove_control::Server>>,
}

/// A project was opened from the command line: the UI switches to the editor.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct RemoteOpened(pub ProjectView);

/// The UI reports its selection and viewer mode (for `ove app context`).
#[tauri::command]
#[specta::specta]
pub fn report_ui_state(control: State<'_, Control>, ui: UiState) -> AppResult<()> {
    *relock(&control.ui) = ui;
    Ok(())
}

fn refusal(code: &str, message: impl Into<String>) -> ove_control::Refusal {
    ove_control::Refusal::new(code, message)
}

fn same_bundle(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

fn context(app: &AppHandle) -> Value {
    let state = app.state::<AppState>();
    let project = lock(&state.projects.open).ok().and_then(|g| {
        g.as_ref().map(|p| {
            json!({"id": p.doc().project.id, "name": p.doc().project.name, "bundle": p.bundle(), "rev": p.rev(),
                   "aspectRatio": p.doc().project.aspect_ratio, "durationSeconds": p.doc().duration() as f64 / 1e6})
        })
    });
    let pb = relock(&state.engine.shared.state).clone();
    let fps = if pb.frame_rate > 0.0 { pb.frame_rate } else { 30.0 };
    let ui = relock(&app.state::<Control>().ui).clone();
    json!({
        "app": {"version": env!("CARGO_PKG_VERSION")},
        "project": project,
        "playback": {"seconds": pb.frame as f64 / fps, "frame": pb.frame, "frameRate": fps, "playing": pb.playing, "hasMedia": pb.has_media},
        "selection": ui.selection,
        "viewer": if ui.viewer.is_empty() { "timeline".to_string() } else { ui.viewer },
    })
}

fn transport(app: &AppHandle, cmd: TransportCommand) -> Result<Value, ove_control::Refusal> {
    let state = app.state::<AppState>();
    state.engine.send(crate::engine::EngineMsg::Transport(cmd)).map_err(|e| refusal("app.engine", e.message))?;
    Ok(json!({}))
}

/// The open project, checked against the bundle and revision the caller computed its change from.
fn check_open(p: Option<&ove_store::project::Project>, params: &Value) -> Result<(), ove_control::Refusal> {
    let p = p.ok_or_else(|| refusal("app.noProject", "the app has no project open"))?;
    let bundle = params.get("bundle").and_then(Value::as_str).unwrap_or_default();
    if !same_bundle(Path::new(bundle), p.bundle()) {
        return Err(refusal("app.otherProject", format!("the app has {} open", p.doc().project.name)));
    }
    if let Some(r) = params.get("expectRev").and_then(Value::as_i64)
        && r != p.rev()
    {
        return Err(refusal("rev.changed", "the project changed in the app; recompute and try again"));
    }
    Ok(())
}

/// The revision source of a change sent over the control link. Only the command line and its MCP server use the
/// link, so only `cli` and `mcp` are accepted (`cli` when absent): a caller cannot pass its change off as the
/// editor's (`gui`), the in-app assistant's (`agent`, which the assistant's own undo acts on) or a history step.
fn socket_source(params: &Value) -> Result<Source, ove_control::Refusal> {
    let Some(v) = params.get("source").filter(|v| !v.is_null()) else { return Ok(Source::Cli) };
    match serde_json::from_value::<Source>(v.clone()) {
        Ok(s @ (Source::Cli | Source::Mcp)) => Ok(s),
        _ => Err(refusal("source.invalid", format!("changes over the control link come from cli or mcp, not {v}"))),
    }
}

fn handle(app: &AppHandle, method: &str, params: Value) -> Result<Value, ove_control::Refusal> {
    match method {
        "context" => Ok(context(app)),
        "open" => {
            let bundle =
                params.get("bundle").and_then(Value::as_str).ok_or_else(|| refusal("argument.missing", "bundle"))?;
            // Takes its turn with the editor's own project commands.
            let v = tauri::async_runtime::block_on(crate::projects::open_project(app.clone(), bundle.to_string()))
                .map_err(|e| refusal("project.open", e.message))?;
            let _ = RemoteOpened(v).emit(app);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            Ok(context(app))
        }
        "seek" => {
            let s = params.get("seconds").and_then(Value::as_f64).filter(|s| s.is_finite() && *s >= 0.0);
            let s = s.ok_or_else(|| refusal("argument.invalid", "seconds"))?;
            let fps = relock(&app.state::<AppState>().engine.shared.state).frame_rate;
            let frame = (s * if fps > 0.0 { fps } else { 30.0 }).round() as u32;
            transport(app, TransportCommand::SeekFrame { frame })
        }
        "play" => transport(app, TransportCommand::Play),
        "pause" => transport(app, TransportCommand::Pause),
        "apply" => {
            let ops: Vec<Op> = serde_json::from_value(params.get("ops").cloned().unwrap_or(Value::Null))
                .map_err(|e| refusal("ops.invalid", e.to_string()))?;
            let source = socket_source(&params)?;
            let summary = params.get("summary").and_then(Value::as_str).map(str::to_string);
            let state = app.state::<AppState>();
            let mut g = lock(&state.projects.open).map_err(|e| refusal("app.busy", e.message))?;
            check_open(g.as_ref(), &params)?;
            let p = g.as_mut().expect("checked");
            p.apply(ops, source, summary).map_err(|e| refusal(e.code(), e.to_string()))?;
            emit(app, Some(p));
            Ok(json!({"rev": p.rev()}))
        }
        "restore" => {
            let rev = params.get("rev").and_then(Value::as_i64).ok_or_else(|| refusal("argument.missing", "rev"))?;
            let state = app.state::<AppState>();
            let mut g = lock(&state.projects.open).map_err(|e| refusal("app.busy", e.message))?;
            check_open(g.as_ref(), &params)?;
            let p = g.as_mut().expect("checked");
            let c = p.restore_as(rev, Source::Undo).map_err(|e| refusal(e.code(), e.to_string()))?;
            emit(app, Some(p));
            Ok(match c {
                Some(c) => json!({"rev": c.record.rev, "summary": c.record.summary}),
                None => json!({"rev": null}),
            })
        }
        other => Err(refusal("method.unknown", format!("unknown method {other}"))),
    }
}

/// Starts the control link for the app's library. Failure only disables the link (logged).
pub fn start(app: &AppHandle) {
    app.manage(Control::default());
    let root = match ove_store::library::resolve_library(None) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "no library; the command line cannot reach the app");
            return;
        }
    };
    let h = app.clone();
    match ove_control::serve(&root, env!("CARGO_PKG_VERSION"), std::sync::Arc::new(move |m, p| handle(&h, m, p))) {
        Ok(s) => {
            *relock(&app.state::<Control>().server) = Some(s);
        }
        Err(e) => tracing::warn!(error = %e, "control link could not start"),
    }
}

/// Stops the link and removes the endpoint file.
pub fn stop(app: &AppHandle) {
    if let Some(c) = app.try_state::<Control>() {
        relock(&c.server).take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_changes_are_cli_or_mcp_only() {
        assert_eq!(socket_source(&json!({})).unwrap(), Source::Cli);
        assert_eq!(socket_source(&json!({"source": null})).unwrap(), Source::Cli);
        assert_eq!(socket_source(&json!({"source": "cli"})).unwrap(), Source::Cli);
        assert_eq!(socket_source(&json!({"source": "mcp"})).unwrap(), Source::Mcp);
        for spoofed in ["gui", "agent", "undo", "redo", "restore", "migration", "nonsense"] {
            let e = socket_source(&json!({"source": spoofed})).unwrap_err();
            assert_eq!(e.code, "source.invalid", "{spoofed}");
        }
        assert!(socket_source(&json!({"source": 3})).is_err());
    }
}
