//! The Transcript panel (add-transcript-editing 5.4): the words on the timeline, search, filler and pause review,
//! and cuts. Every action runs the same registry tool agents use (`get_transcript`, `search_transcript`,
//! `find_fillers`, `remove_words`, `remove_silence`) on the project open in the editor, as the person (source `gui`),
//! so a cut is one revision and one undo step.

use serde_json::{Map, Value};
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, emit_view, lock, serial};

/// The tools the panel may run, and whether each may change the project.
const TOOLS: &[(&str, bool)] = &[
    ("get_transcript", false),
    ("search_transcript", false),
    ("find_fillers", false),
    ("remove_words", true),
    ("remove_silence", true),
];

struct Open<'a> {
    app: &'a AppHandle,
    state: &'a crate::AppState,
    view: Option<ProjectView>,
}

impl ove_tools::AppHost for Open<'_> {
    fn with_open(
        &mut self,
        edit: bool,
        f: &mut dyn FnMut(&mut ove_store::Project) -> Result<(), ove_tools::Fail>,
    ) -> Result<(), ove_tools::Fail> {
        let mut g = lock(&self.state.projects.open).map_err(|e| ove_tools::Fail::new("app.busy", e.message))?;
        let p = g.as_mut().ok_or_else(|| ove_tools::Fail::new("app.noProject", "No project is open."))?;
        let before = p.rev();
        let out = f(p);
        if edit && p.rev() != before {
            self.view = emit_view(self.app, Some(p));
        }
        out
    }
}

fn args_of(args: &str) -> AppResult<Map<String, Value>> {
    match serde_json::from_str::<Value>(args) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(AppError::new(ErrorCode::InvalidArgument, "The arguments must be a JSON object.")),
    }
}

fn run(
    app: &AppHandle,
    state: &crate::AppState,
    tool: &str,
    args: &Map<String, Value>,
) -> AppResult<(Value, Option<ProjectView>)> {
    let mut host = Open { app, state, view: None };
    let mut ctx = ove_tools::AppCtx {
        host: &mut host,
        source: ove_core::revision::Source::Gui,
        surface: ove_tools::Surface::InApp,
        actor: None,
    };
    let r = ove_tools::call(&mut ctx, tool, args).map_err(|f| {
        let code = if f.code.starts_with("model.") || f.code == "media.unavailable" {
            ErrorCode::Unsupported
        } else {
            ErrorCode::InvalidArgument
        };
        AppError::new(code, f.message)
    })?;
    Ok((r.json, host.view))
}

/// Runs a read-only transcript tool (or a cutting tool with `apply: false`, a dry run) and returns its JSON result.
#[tauri::command]
#[specta::specta]
pub async fn transcript_query(app: AppHandle, tool: String, args: String) -> AppResult<String> {
    serial(&app, "transcript_query", move |app, state| {
        let mut a = args_of(&args)?;
        match TOOLS.iter().find(|(n, _)| *n == tool) {
            Some((_, true)) => {
                a.insert("apply".into(), Value::Bool(false));
            }
            Some((_, false)) => {}
            None => return Err(AppError::new(ErrorCode::InvalidArgument, format!("{tool} is not a transcript tool"))),
        }
        Ok(run(app, state, &tool, &a)?.0.to_string())
    })
    .await
}

/// Runs a cutting tool (`remove_words`, `remove_silence`) on the open project as one revision.
#[tauri::command]
#[specta::specta]
pub async fn transcript_cut(app: AppHandle, tool: String, args: String) -> AppResult<ProjectView> {
    serial(&app, "transcript_cut", move |app, state| {
        if !TOOLS.iter().any(|(n, cuts)| *n == tool && *cuts) {
            return Err(AppError::new(ErrorCode::InvalidArgument, format!("{tool} is not a cutting tool")));
        }
        let mut a = args_of(&args)?;
        a.insert("apply".into(), Value::Bool(true));
        let (_, view) = run(app, state, &tool, &a)?;
        match view {
            Some(v) => Ok(v),
            // Nothing was cut (no dead air, no speech): the project as it is.
            None => {
                let g = lock(&state.projects.open)?;
                let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
                Ok(crate::projects::view(p))
            }
        }
    })
    .await
}
