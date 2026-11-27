//! History by agent (change add-agent-chat-context-and-activity, design D6): "Undo this agent's changes" in the
//! History panel. It runs the registry's actor-scoped undo (`undo_my_changes` with scope `any`, the command line's
//! form, here on the project open in the app) — the same path agents use for their own changes — so it reverts only
//! that agent's revisions as one new revision, and refuses, changing nothing, when later edits by others changed the
//! same fields.

use ove_core::revision::Source;
use ove_store::project::Project;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use specta::Type;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, emit_view, lock, serial};

/// A field an agent changed that someone changed again later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UndoConflictRow {
    /// The item (its id), or the track, media or project.
    pub item: String,
    pub field: String,
    /// The later change, as History lists it.
    pub summary: String,
}

/// What "Undo this agent's changes" did.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentUndo {
    /// The project after the undo; none when it was refused.
    pub view: Option<ProjectView>,
    /// Why it was refused: the fields changed later by others (nothing was changed).
    pub conflicts: Vec<UndoConflictRow>,
}

/// Why an agent's changes were not undone.
#[derive(Debug)]
pub enum Refused {
    Conflicts(Vec<UndoConflictRow>),
    Other(AppError),
}

/// Reverts the revisions of one agent session (`kind`: mcp, cli or assistant), or of one of its turns, as one new
/// revision of the person's.
pub fn undo_agent(p: &mut Project, kind: &str, session: &str, turn: Option<&str>) -> Result<i64, Refused> {
    if !matches!(kind, "mcp" | "cli" | "assistant") {
        return Err(Refused::Other(AppError::new(
            ErrorCode::InvalidArgument,
            "Only an agent's changes can be undone here.",
        )));
    }
    let mut args = Map::new();
    args.insert("scope".into(), "any".into());
    args.insert("kind".into(), kind.into());
    args.insert("session".into(), session.into());
    if let Some(t) = turn {
        args.insert("turn".into(), t.into());
    }
    let mut host = ove_tools::ProjectHost { project: p };
    let mut ctx =
        ove_tools::AppCtx { host: &mut host, source: Source::Gui, surface: ove_tools::Surface::Cli, actor: None };
    match ove_tools::call(&mut ctx, "undo_my_changes", &args) {
        Ok(r) => Ok(r.json["rev"].as_i64().unwrap_or_default()),
        Err(f) if f.code == "undo.conflict" => Err(Refused::Conflicts(
            f.details["conflicts"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .map(|x| UndoConflictRow {
                            item: x["item"].as_str().or(x["object"].as_str()).unwrap_or_default().to_string(),
                            field: x["field"].as_str().unwrap_or_default().to_string(),
                            summary: x["summary"].as_str().unwrap_or_default().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        )),
        Err(f) if f.code == "undo.nothing" => {
            Err(Refused::Other(AppError::new(ErrorCode::InvalidArgument, "This agent's changes are already undone.")))
        }
        Err(f) => Err(Refused::Other(AppError::new(ErrorCode::InvalidArgument, f.message))),
    }
}

/// History → "Undo this agent's changes": every revision of that agent's session (or of one assistant turn), as one
/// new revision that can itself be undone. Refused when later edits by others changed the same fields: nothing
/// changes, and the conflicting items are listed.
#[tauri::command]
#[specta::specta]
pub async fn undo_agent_changes(
    app: tauri::AppHandle,
    kind: String,
    session: String,
    turn: Option<String>,
) -> AppResult<AgentUndo> {
    serial(&app, "undo_agent_changes", move |app, state| {
        let mut g = lock(&state.projects.open)?;
        let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        match undo_agent(p, &kind, &session, turn.as_deref()) {
            Ok(_) => Ok(AgentUndo { view: emit_view(app, Some(p)), conflicts: vec![] }),
            Err(Refused::Conflicts(conflicts)) => Ok(AgentUndo { view: None, conflicts }),
            Err(Refused::Other(e)) => Err(e),
        }
    })
    .await
}

/// The values a history row groups by: the actor's kind, session and turn, when recorded.
pub fn actor_fields(actor: Option<&ove_core::revision::Actor>) -> (Option<String>, Option<String>, Option<String>) {
    match actor {
        Some(a) => (Some(a.kind.as_str().to_string()), a.session.clone(), a.turn.clone()),
        None => (None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::Op;
    use ove_core::revision::{Actor, ActorKind};
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn project() -> (tempfile::TempDir, Project) {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("H").unwrap();
        p.apply(
            vec![
                Op::AddTrack { track: json!({"id": "t", "name": "Main", "order": 0, "type": "video"}) },
                Op::AddItem { item: json!({"id": "a", "trackId": "t", "type": "text", "startTime": 0, "endTime": 1, "textContent": "A"}) },
                Op::AddItem { item: json!({"id": "b", "trackId": "t", "type": "text", "startTime": 1, "endTime": 2, "textContent": "B"}) },
            ],
            Source::Gui,
            None,
        )
        .unwrap();
        (d, p)
    }

    fn set(k: &str, v: Value) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(k.into(), v);
        m
    }

    fn agent() -> Option<Actor> {
        Some(Actor { kind: ActorKind::Mcp, session: Some("s1".into()), turn: None, client: Some("claude-code".into()) })
    }

    #[test]
    fn undoing_one_agent_keeps_the_persons_edits() {
        let (_d, mut p) = project();
        p.apply_as(
            vec![Op::UpdateItem { id: "a".into(), set: set("textContent", json!("A2")) }],
            Source::Mcp,
            None,
            agent(),
        )
        .unwrap();
        p.apply(vec![Op::UpdateItem { id: "b".into(), set: set("textContent", json!("B2")) }], Source::Gui, None)
            .unwrap();
        p.apply_as(
            vec![Op::UpdateItem { id: "a".into(), set: set("opacity", json!(0.5)) }],
            Source::Mcp,
            None,
            agent(),
        )
        .unwrap();
        undo_agent(&mut p, "mcp", "s1", None).map_err(|_| "refused").unwrap();
        let doc = p.doc();
        assert_eq!(doc.project.items["a"].fields["textContent"], "A");
        assert!(doc.project.items["a"].fields.get("opacity").is_none());
        assert_eq!(doc.project.items["b"].fields["textContent"], "B2", "the person's edit stays");
        let last = p.latest_record().unwrap().unwrap();
        assert_eq!(last.source, Source::Undo);
        assert!(p.session().can_undo(), "the undo is itself one undo step");
        // Twice: nothing left.
        assert!(matches!(undo_agent(&mut p, "mcp", "s1", None), Err(Refused::Other(_))));
    }

    #[test]
    fn a_later_edit_to_the_same_field_is_a_conflict_that_changes_nothing() {
        let (_d, mut p) = project();
        p.apply_as(
            vec![Op::UpdateItem { id: "a".into(), set: set("textContent", json!("A2")) }],
            Source::Mcp,
            None,
            agent(),
        )
        .unwrap();
        p.apply(
            vec![Op::UpdateItem { id: "a".into(), set: set("textContent", json!("Mine")) }],
            Source::Gui,
            Some("edit text".into()),
        )
        .unwrap();
        let rev = p.rev();
        let Err(Refused::Conflicts(c)) = undo_agent(&mut p, "mcp", "s1", None) else { panic!("not refused") };
        assert_eq!(c, [UndoConflictRow { item: "a".into(), field: "textContent".into(), summary: "edit text".into() }]);
        assert_eq!(p.rev(), rev, "nothing changed");
        assert_eq!(p.doc().project.items["a"].fields["textContent"], "Mine");
        assert!(matches!(undo_agent(&mut p, "user", "", None), Err(Refused::Other(_))));
    }
}
