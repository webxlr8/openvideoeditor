//! Live agent activity (change add-agent-chat-context-and-activity, design D5): what the in-app assistant and
//! external agents read and change on the open project, sent to the timeline as `agentActivity`.
//!
//! Reads come from the tool registry (`ove_tools::call` tells its context; the assistant's context emits here, and
//! external agents send the control-link method `notify`). Changes come from the project's own revisions: after each
//! change the app looks at the revisions it has not shown yet, so every edit — the assistant's, one sent over the
//! control link, an undo — is shown from what it actually did. The person's own edits are sent as `person`, so the
//! timeline drops agent outlines on items the person just changed. Nothing here leaves the machine.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ove_core::revision::{Actor, ActorKind, Source};
use ove_tools::activity::{self, Activity, Notice};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::AppState;
use crate::worker::relock;

/// Revisions shown per change at most (a burst of many is shown as its latest ones).
const MAX_REVISIONS: usize = 50;

/// One thing an agent did on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivity {
    /// Who, as a stable key: the actor kind and its session (`mcp:<session>`, `assistant:<chat>`).
    pub actor: String,
    /// The badge's name: Assistant, the client's name (Claude Code), MCP agent or Command line.
    pub name: String,
    /// `read`, `change`, or `person` (the person's own edit; never outlined).
    pub kind: String,
    /// Items read, changed or added.
    pub ids: Vec<String>,
    /// Items removed.
    pub removed: Vec<String>,
    /// The time range read, seconds.
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// When, ms since the epoch.
    pub ts: f64,
}

/// The project and revision the display last caught up with.
static SEEN: Mutex<Option<(PathBuf, i64)>> = Mutex::new(None);

fn now_ms() -> f64 {
    ove_core::Clock::now_ms(&ove_core::SystemClock) as f64
}

/// The event for an activity by `actor`.
pub fn event(actor: Option<&Actor>, source: Source, kind: &str, a: &Activity) -> AgentActivity {
    let (key, name) = activity::actor_label(actor, source);
    AgentActivity {
        actor: key,
        name,
        kind: kind.to_string(),
        ids: a.ids.clone(),
        removed: a.removed.clone(),
        start: a.start,
        end: a.end,
        ts: now_ms(),
    }
}

/// Shows what `actor` does (`a.kind`: read or change).
pub fn emit(app: &AppHandle, actor: Option<&Actor>, source: Source, a: &Activity) {
    if a.is_empty() {
        return;
    }
    let _ = event(actor, source, &a.kind, a).emit(app);
}

/// The events for the revisions committed since the display last caught up with `bundle` at `seen` (none the first
/// time a project is seen).
pub fn changes(bundle: &Path, seen: Option<i64>, head: i64) -> Vec<AgentActivity> {
    let Some(seen) = seen.filter(|s| *s != head) else { return vec![] };
    activity::since(bundle, seen, head, MAX_REVISIONS)
        .into_iter()
        .filter_map(|r| {
            let a = activity::of_ops(&r.ops);
            if a.is_empty() {
                return None;
            }
            let kind = if activity::by_agent(&r) { activity::CHANGE } else { "person" };
            Some(event(r.actor.as_ref(), r.source, kind, &a))
        })
        .collect()
}

/// After the open project changed (called with every project view the app sends): shows the new revisions.
pub fn after_change(app: &AppHandle, p: &ove_store::project::Project) {
    let (bundle, head) = (p.bundle().to_path_buf(), p.rev());
    let seen = {
        let mut g = relock(&SEEN);
        let seen = g.as_ref().filter(|(b, _)| *b == bundle).map(|(_, r)| *r);
        *g = Some((bundle.clone(), head));
        seen
    };
    for e in changes(&bundle, seen, head) {
        let _ = e.emit(app);
    }
}

/// The control-link method `notify`: an external agent says what it reads. Shown only when it is about the open
/// project; only reads by command-line and MCP actors are accepted (changes are shown from the revisions themselves).
pub fn notify(app: &AppHandle, params: Value) -> Result<Value, ove_control::Refusal> {
    let n: Notice = serde_json::from_value(params)
        .map_err(|e| ove_control::Refusal::new("argument.invalid", format!("notify: {e}")))?;
    if let Some(a) = &n.actor
        && !matches!(a.kind, ActorKind::Cli | ActorKind::Mcp)
    {
        return Err(ove_control::Refusal::new("actor.invalid", "notifications come from cli or mcp actors"));
    }
    if n.activity.kind != activity::READ {
        return Err(ove_control::Refusal::new("argument.invalid", "notify reports reads only"));
    }
    let state = app.state::<AppState>();
    let open =
        crate::projects::lock(&state.projects.open).ok().and_then(|g| g.as_ref().map(|p| p.bundle().to_path_buf()));
    let shown = open.is_some_and(|b| ove_tools::bundle::same_bundle(&b, &n.bundle));
    if shown {
        let source = if n.actor.as_ref().is_some_and(|a| a.kind == ActorKind::Mcp) { Source::Mcp } else { Source::Cli };
        emit(app, n.actor.as_ref(), source, &n.activity);
    }
    Ok(json!({"shown": shown}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::Op;
    use std::sync::Arc;

    #[test]
    fn agent_revisions_are_changes_and_the_persons_are_not() {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("A").unwrap();
        p.apply(
            vec![
                Op::AddTrack { track: json!({"id": "t", "name": "Main", "order": 0, "type": "video"}) },
                Op::AddItem { item: json!({"id": "a", "trackId": "t", "type": "text", "startTime": 0, "endTime": 1}) },
            ],
            Source::Gui,
            None,
        )
        .unwrap();
        let seen = p.rev();
        assert!(changes(p.bundle(), None, seen).is_empty(), "nothing is shown for a project seen the first time");
        let agent =
            Actor { kind: ActorKind::Mcp, session: Some("s".into()), turn: None, client: Some("claude-code".into()) };
        let mut set = serde_json::Map::new();
        set.insert("opacity".into(), json!(0.5));
        p.apply_as(vec![Op::UpdateItem { id: "a".into(), set: set.clone() }], Source::Mcp, None, Some(agent)).unwrap();
        p.apply(vec![Op::RemoveItems { ids: vec!["a".into()] }], Source::Gui, None).unwrap();
        let ev = changes(p.bundle(), Some(seen), p.rev());
        assert_eq!(ev.len(), 2);
        assert_eq!(
            (ev[0].kind.as_str(), ev[0].name.as_str(), ev[0].actor.as_str()),
            ("change", "Claude Code", "mcp:s")
        );
        assert_eq!(ev[0].ids, ["a"]);
        assert_eq!((ev[1].kind.as_str(), ev[1].name.as_str()), ("person", "You"));
        assert_eq!(ev[1].removed, ["a"]);
        assert!(changes(p.bundle(), Some(p.rev()), p.rev()).is_empty());
    }
}
