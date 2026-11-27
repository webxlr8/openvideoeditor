//! Named versions in the History panel (add-markers-keyframe-lanes-and-versions task 4.5): list, save, compare with
//! the current timeline or another version, and restore as one undoable revision.

use ove_core::revision::{Actor, ActorKind};
use ove_store::versions::Version;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};

/// A version as the History panel shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VersionView {
    pub name: String,
    pub rev: f64,
    /// ms since the epoch.
    pub created_at: f64,
    pub note: Option<String>,
    /// Who saved it, in words.
    pub who: String,
    /// Bytes it added to the bundle.
    pub bytes: f64,
}

/// One classified change between two versions.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VersionChange {
    /// `item` or `marker`.
    pub entity: String,
    pub id: String,
    /// added, removed, moved, trimmed or changed.
    pub class: String,
    /// The change in words, with old and new times.
    pub line: String,
}

fn who(a: Option<&Actor>) -> String {
    match a.map(|a| a.kind) {
        Some(ActorKind::Assistant) => "Assistant",
        Some(ActorKind::Mcp) => "MCP",
        Some(ActorKind::Cli) => "Command line",
        _ => "You",
    }
    .into()
}

fn view(v: &Version) -> VersionView {
    VersionView {
        name: v.name.clone(),
        rev: v.rev as f64,
        created_at: v.created_at as f64,
        note: v.note.clone(),
        who: who(v.actor.as_ref()),
        bytes: v.bytes as f64,
    }
}

fn no_project() -> AppError {
    AppError::new(ErrorCode::InvalidArgument, "No project is open.")
}

fn refused(e: ove_store::StoreError) -> AppError {
    let code = if e.code() == "version.missing" { ErrorCode::NotFound } else { ErrorCode::InvalidArgument };
    AppError::new(code, e.to_string())
}

/// The open project's versions, newest first.
#[tauri::command]
#[specta::specta]
pub async fn version_list(app: AppHandle) -> AppResult<Vec<VersionView>> {
    serial(&app, "version_list", |_, state| {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(no_project)?;
        Ok(p.versions().map_err(refused)?.iter().rev().map(view).collect())
    })
    .await
}

/// Saves the current revision as a named version and returns the list, newest first.
#[tauri::command]
#[specta::specta]
pub async fn version_save(app: AppHandle, name: String, note: Option<String>) -> AppResult<Vec<VersionView>> {
    serial(&app, "version_save", move |_, state| {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(no_project)?;
        let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
        p.save_version(&name, note.as_deref(), Some(Actor::new(ActorKind::User)), now).map_err(refused)?;
        Ok(p.versions().map_err(refused)?.iter().rev().map(view).collect())
    })
    .await
}

/// The changes from version `from` to version `to`, or to the current timeline when `to` is absent.
#[tauri::command]
#[specta::specta]
pub async fn version_diff(app: AppHandle, from: String, to: Option<String>) -> AppResult<Vec<VersionChange>> {
    serial(&app, "version_diff", move |_, state| {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(no_project)?;
        let changes = p.version_diff(&from, to.as_deref()).map_err(refused)?;
        Ok(changes
            .iter()
            .map(|c| VersionChange {
                entity: c.entity.to_string(),
                id: c.id.clone(),
                class: c.class.as_str().to_string(),
                line: c.line(),
            })
            .collect())
    })
    .await
}

/// Makes the timeline equal to a version again, as one new revision that Undo reverts.
#[tauri::command]
#[specta::specta]
pub async fn version_restore(app: AppHandle, name: String) -> AppResult<ProjectView> {
    serial(&app, "version_restore", move |app, state| {
        let ops = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(no_project)?;
            let target = p.version_doc(&name).map_err(refused)?;
            ove_core::diff::diff_documents(p.doc(), &target)
        };
        edit(app, state, ops, &format!("restore version \u{201c}{name}\u{201d}"))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_names_the_saver() {
        assert_eq!(who(None), "You");
        assert_eq!(who(Some(&Actor::new(ActorKind::Mcp))), "MCP");
        let v = Version { name: "v1".into(), rev: 5, created_at: 9, actor: None, note: Some("n".into()), bytes: 10 };
        assert_eq!(view(&v).who, "You");
        assert_eq!(view(&v).rev, 5.0);
    }
}
