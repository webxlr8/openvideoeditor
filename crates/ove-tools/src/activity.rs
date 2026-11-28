//! What an agent is reading or changing, for the live display on the timeline (change
//! add-agent-chat-context-and-activity, design D5). A read tool maps its arguments or receipt to items or a time range
//! (a printed line range to the items on those lines, `look` to the times it renders); a change is the items a
//! revision's operations touched. The running app shows them; nothing here leaves the machine.

use std::path::{Path, PathBuf};

use ove_core::ops::Op;
use ove_core::revision::{Actor, ActorKind, RevisionRecord, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::receipt::Receipt;

/// `read` or `change`.
pub const READ: &str = "read";
pub const CHANGE: &str = "change";

/// One thing an agent did on the timeline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// [`READ`] or [`CHANGE`].
    pub kind: String,
    /// Items read, changed or added.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Items removed (a change).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// The time range read, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

impl Activity {
    fn read_ids(ids: Vec<String>) -> Activity {
        Activity { kind: READ.into(), ids, ..Activity::default() }
    }

    fn read_range(start: f64, end: f64) -> Activity {
        Activity { kind: READ.into(), start: Some(start.min(end)), end: Some(start.max(end)), ..Activity::default() }
    }

    /// True when it names nothing.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.removed.is_empty() && self.start.is_none()
    }
}

/// The read `notify` sends over the control link: who read what, in which project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub bundle: PathBuf,
    #[serde(default)]
    pub actor: Option<Actor>,
    pub activity: Activity,
}

/// The tools whose reads are shown.
pub fn watched(tool: &str) -> bool {
    matches!(tool, "print_timeline" | "look" | "get_transcript")
}

/// Times in a comma-separated `at` argument.
fn times(at: &str) -> Vec<f64> {
    at.split(',').filter_map(|t| t.trim().parse::<f64>().ok()).filter(|t| t.is_finite() && *t >= 0.0).collect()
}

fn span(ts: &[f64]) -> Option<Activity> {
    let lo = ts.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lo.is_finite() && hi.is_finite()).then(|| Activity::read_range(lo, hi))
}

/// What a call reads, known from its arguments before it runs (so the display shows it while it runs): `look`
/// with `at` times.
pub fn before(tool: &str, args: &Map<String, Value>) -> Option<Activity> {
    match tool {
        "look" => span(&times(args.get("at")?.as_str()?)),
        _ => None,
    }
}

/// The timeline duration from a printed header (`duration: 12.5s`).
fn printed_duration(text: &str) -> Option<f64> {
    text.lines()
        .take_while(|l| !l.trim_start().starts_with("*/"))
        .find_map(|l| l.trim_start().trim_start_matches('*').trim().strip_prefix("duration:"))
        .and_then(|d| d.trim().trim_end_matches('s').parse().ok())
}

/// What a finished call read, from its receipt: the items on the printed lines of a partial `print_timeline` (the
/// whole timeline for a full print), the times `look` rendered, the clips `get_transcript` read.
pub fn after(tool: &str, args: &Map<String, Value>, r: &Receipt) -> Option<Activity> {
    let a = match tool {
        "print_timeline" => {
            let text = r.json.get("text")?.as_str()?;
            if r.json.get("range").is_some() {
                Activity::read_ids(crate::edits::element_spans(text).into_iter().map(|s| s.id).collect())
            } else {
                Activity::read_range(0.0, printed_duration(text)?)
            }
        }
        "look" => {
            let ts: Vec<f64> = r.json.get("frames")?.as_array()?.iter().filter_map(|f| f["at"].as_f64()).collect();
            span(&ts)?
        }
        "get_transcript" => {
            let clips: Vec<String> =
                args.get("clips")?.as_array()?.iter().filter_map(|c| c.as_str().map(str::to_string)).collect();
            Activity::read_ids(clips)
        }
        _ => return None,
    };
    (!a.is_empty()).then_some(a)
}

/// The items a batch of operations added or changed, and the ones it removed.
pub fn of_ops(ops: &[Op]) -> Activity {
    let mut ids: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let add = |v: &mut Vec<String>, id: &str| {
        if !v.iter().any(|x| x == id) {
            v.push(id.to_string());
        }
    };
    for op in ops {
        match op {
            Op::AddItem { item } => {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    removed.retain(|x| x != id);
                    add(&mut ids, id);
                }
            }
            Op::UpdateItem { id, .. } | Op::UnsetItemFields { id, .. } => add(&mut ids, id),
            Op::RemoveItems { ids: gone } => {
                for id in gone {
                    ids.retain(|x| x != id);
                    add(&mut removed, id);
                }
            }
            _ => {}
        }
    }
    Activity { kind: CHANGE.into(), ids, removed, ..Activity::default() }
}

/// True when a revision is an agent's (the in-app assistant, an MCP agent or the command line), not the person's.
pub fn by_agent(r: &RevisionRecord) -> bool {
    match &r.actor {
        Some(a) => matches!(a.kind, ActorKind::Assistant | ActorKind::Mcp | ActorKind::Cli),
        None => matches!(r.source, Source::Agent | Source::Mcp | Source::Cli),
    }
}

/// The revisions after `seen` up to `head`, oldest first, following each revision's parent (at most `limit`).
pub fn since(bundle: &Path, seen: i64, head: i64, limit: usize) -> Vec<RevisionRecord> {
    let mut out = Vec::new();
    let mut rev = head;
    while rev != seen && out.len() < limit {
        let Ok(r) = ove_store::project::read_record(bundle, rev) else { break };
        rev = r.parent;
        out.push(r);
    }
    out.reverse();
    out
}

/// A client name as people read it: `claude-code` → `Claude Code`.
pub fn client_name(client: &str) -> String {
    client
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Who an activity is shown as: a stable key (kind and session) and a name for the badge.
pub fn actor_label(actor: Option<&Actor>, source: Source) -> (String, String) {
    let kind = actor.map(|a| a.kind).unwrap_or(match source {
        Source::Agent => ActorKind::Assistant,
        Source::Mcp => ActorKind::Mcp,
        Source::Cli => ActorKind::Cli,
        _ => ActorKind::User,
    });
    let name = match (kind, actor.and_then(|a| a.client.as_deref())) {
        (ActorKind::Assistant, _) => "Assistant".to_string(),
        (_, Some(c)) if !c.trim().is_empty() => client_name(c),
        (ActorKind::Mcp, _) => "MCP agent".to_string(),
        (ActorKind::Cli, _) => "Command line".to_string(),
        (ActorKind::Generation, _) => "Generation".to_string(),
        (ActorKind::Template, _) => "Template".to_string(),
        _ => "You".to_string(),
    };
    let session = actor.and_then(|a| a.session.as_deref()).unwrap_or_default();
    (format!("{}:{session}", kind.as_str()), name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    const TEXT: &str = "/*\n * duration: 12.5s\n * rev: 4\n */\nexport default (\n  <timeline aspect=\"16:9\">\n    <track id=\"t\" name=\"T\" kind=\"video\">\n      <text id=\"a\" name=\"A\" start={0} end={1} />\n      <text\n        id=\"b\"\n        name=\"B\"\n      />\n    </track>\n  </timeline>\n);\n";

    #[test]
    fn a_printed_line_range_maps_to_its_items_and_a_full_print_to_the_whole_timeline() {
        let part = Receipt::new(
            json!({"rev": 4, "text": "      <text\n        id=\"b\"\n        name=\"B\"\n      />", "range": {"from": 9, "to": 12, "total": 15}}),
            "",
        );
        assert_eq!(after("print_timeline", &Map::new(), &part).unwrap().ids, ["b"]);
        let full = Receipt::new(json!({"rev": 4, "text": TEXT}), "");
        let a = after("print_timeline", &Map::new(), &full).unwrap();
        assert_eq!((a.kind.as_str(), a.start, a.end), (READ, Some(0.0), Some(12.5)));
        assert!(a.ids.is_empty());
    }

    #[test]
    fn look_maps_to_the_times_it_renders_before_and_after_it_runs() {
        let a = before("look", &args(json!({"at": "20, 10,15"}))).unwrap();
        assert_eq!((a.start, a.end), (Some(10.0), Some(20.0)));
        assert!(before("look", &args(json!({"count": 6}))).is_none());
        let r = Receipt::new(json!({"frames": [{"at": 2.0}, {"at": 8.5}]}), "");
        let a = after("look", &Map::new(), &r).unwrap();
        assert_eq!((a.start, a.end), (Some(2.0), Some(8.5)));
        let t = after("get_transcript", &args(json!({"clips": ["c1", "c2"]})), &Receipt::new(json!({}), "")).unwrap();
        assert_eq!(t.ids, ["c1", "c2"]);
        assert!(after("list_media", &Map::new(), &Receipt::new(json!({}), "")).is_none());
    }

    #[test]
    fn a_revisions_ops_map_to_changed_and_removed_items() {
        let mut set = Map::new();
        set.insert("opacity".into(), json!(0.5));
        let a = of_ops(&[
            Op::AddItem { item: json!({"id": "n"}) },
            Op::UpdateItem { id: "u".into(), set: set.clone() },
            Op::UpdateItem { id: "u".into(), set },
            Op::RemoveItems { ids: vec!["r".into(), "n".into()] },
            Op::SetAspectRatio { aspect_ratio: "1:1".into() },
        ]);
        assert_eq!(a.kind, CHANGE);
        assert_eq!(a.ids, ["u"]);
        assert_eq!(a.removed, ["r", "n"]);
    }

    #[test]
    fn actors_read_as_names() {
        assert_eq!(client_name("claude-code"), "Claude Code");
        assert_eq!(client_name("my_agent v2"), "My Agent V2");
        let mcp = Actor { kind: ActorKind::Mcp, session: Some("s1".into()), turn: None, client: Some("codex".into()) };
        assert_eq!(actor_label(Some(&mcp), Source::Mcp), ("mcp:s1".to_string(), "Codex".to_string()));
        assert_eq!(actor_label(None, Source::Agent).1, "Assistant");
        assert_eq!(actor_label(None, Source::Cli).1, "Command line");
        assert_eq!(actor_label(None, Source::Gui).1, "You");
    }
}
