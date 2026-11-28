//! Help → Feedback (change unify-agent-tool-layer, D8): the reports agents drafted with `send_feedback`, kept in
//! `<config>/feedback/` on this computer. The person reads, edits, copies or deletes each one. Nothing here sends
//! anything: a report leaves the computer only when the person pastes the copied text somewhere themselves.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

/// One drafted report.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackDraft {
    pub id: String,
    /// Milliseconds since the epoch.
    pub created_at: f64,
    pub category: String,
    pub severity: String,
    pub summary: String,
    /// Who drafted it: the agent's client name, or where it ran.
    pub from: String,
    pub recent_tools: Vec<String>,
    pub last_error: Option<String>,
    pub edited: bool,
    /// The text "Copy report" puts on the clipboard.
    pub report: String,
}

fn view(d: ove_tools::tools::feedback::Draft) -> FeedbackDraft {
    let report = ove_tools::tools::feedback::report_text(&d);
    FeedbackDraft {
        from: d.client.clone().unwrap_or_else(|| d.surface.clone()),
        id: d.id,
        created_at: d.created_at as f64,
        category: d.category,
        severity: d.severity,
        summary: d.summary,
        recent_tools: d.recent_tools,
        last_error: d.last_error,
        edited: d.edited_at.is_some(),
        report,
    }
}

fn config() -> AppResult<std::path::PathBuf> {
    ove_tools::session::config_dir()
        .ok_or_else(|| AppError::new(ErrorCode::Io, "The app has no configuration folder on this computer."))
}

fn fail(f: ove_tools::Fail) -> AppError {
    let code = if f.code == "feedback.notFound" { ErrorCode::NotFound } else { ErrorCode::InvalidArgument };
    AppError::new(code, f.message)
}

/// The drafted reports, newest first.
#[tauri::command]
#[specta::specta]
pub async fn feedback_drafts() -> AppResult<Vec<FeedbackDraft>> {
    worker::blocking("feedback_drafts", || {
        Ok(ove_tools::tools::feedback::list(&config()?).into_iter().map(view).collect())
    })
    .await
}

/// Saves the person's edit of a report's summary.
#[tauri::command]
#[specta::specta]
pub async fn save_feedback_draft(id: String, summary: String) -> AppResult<FeedbackDraft> {
    worker::blocking("save_feedback_draft", move || {
        ove_tools::tools::feedback::update_summary(&config()?, &id, &summary).map(view).map_err(fail)
    })
    .await
}

/// Deletes a report.
#[tauri::command]
#[specta::specta]
pub async fn delete_feedback_draft(id: String) -> AppResult<()> {
    worker::blocking("delete_feedback_draft", move || ove_tools::tools::feedback::delete(&config()?, &id).map_err(fail))
        .await
}
