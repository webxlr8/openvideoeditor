//! Drafting a skill from a finished project (design D6). The context pack holds what an agent needs to write a
//! reusable playbook: the final timeline as printed, its media table (in the printed header), one line per revision
//! grouped by who made it, the presets in use, and the chat when the person includes it. The drafting instructions
//! are our own. The draft is never saved here: the person reviews it and saves it.

use std::collections::BTreeMap;

use ove_store::project::Project;
use serde::Serialize;
use serde_json::Value;

use super::{FrontMatter, parse_front_matter, split_front_matter, validate_text};
use crate::fail::Fail;

/// Most characters of the chat put in a pack.
pub const MAX_CHAT_CHARS: usize = 60_000;
/// Most revision lines per actor.
pub const MAX_HISTORY_LINES: usize = 200;

/// What the pack lists as presets in use, by kind, with how many items use each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Presets(pub BTreeMap<String, BTreeMap<String, usize>>);

impl Presets {
    fn add(&mut self, kind: &str, value: impl Into<String>) {
        let v = value.into();
        if v.trim().is_empty() {
            return;
        }
        *self.0.entry(kind.into()).or_default().entry(v).or_default() += 1;
    }
}

/// The context pack.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pack {
    pub project: String,
    pub timeline: String,
    /// Revision lines by actor (`assistant`, `mcp (client)`, `gui`, …), oldest first.
    pub history: BTreeMap<String, Vec<String>>,
    pub presets: Presets,
    pub chat: Option<String>,
}

fn str_at<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    v.pointer(path).and_then(Value::as_str)
}

/// The presets and looks the project's items use.
pub fn presets(p: &Project) -> Presets {
    let mut out = Presets::default();
    for item in p.doc().project.items.values() {
        let f = Value::Object(item.fields.clone());
        if let Some(id) = str_at(&f, "/captionPresetId") {
            out.add("caption style", id);
        }
        if let Some(id) = str_at(&f, "/textAnimation/preset") {
            out.add("text animation", id);
        }
        for side in ["transitionIn", "transitionOut"] {
            if let Some(t) = str_at(&f, &format!("/{side}/type")) {
                out.add("transition", t);
            }
        }
        if let Some(l) = str_at(&f, "/colorGrade/lut/preset") {
            out.add("LUT look", l);
        }
        if let Some(t) = str_at(&f, "/shaderComposition/template") {
            out.add("shader effect", t);
        }
        if let Some(t) = str_at(&f, "/codeComposition/template") {
            out.add("motion template", t);
        }
        if let Some(Value::Object(g)) = f.get("colorGrading") {
            let set: Vec<String> = g
                .iter()
                .filter(|(_, v)| v.as_f64().is_some_and(|x| x != 0.0))
                .map(|(k, v)| format!("{k} {v}"))
                .collect();
            if !set.is_empty() {
                out.add("colour adjustment", set.join(", "));
            }
        }
        if let Some(l) = f.get("layout").filter(|l| !l.is_null()) {
            out.add("layout slot", l.to_string());
        }
    }
    out
}

/// Builds the pack from a project the caller holds. `chat` is the transcript to include, if any.
pub fn pack(p: &Project, chat: Option<String>) -> Result<Pack, Fail> {
    let mut history: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in p.history()? {
        let who = match &e.actor {
            Some(a) => match &a.client {
                Some(c) => format!("{} ({c})", a.kind.as_str()),
                None => a.kind.as_str().to_string(),
            },
            None => e.source.as_str().to_string(),
        };
        let lines = history.entry(who).or_default();
        if lines.len() < MAX_HISTORY_LINES {
            lines.push(format!("rev {} ({}): {}", e.rev, e.source.as_str(), e.summary));
        }
    }
    let chat = chat.map(|c| {
        if c.chars().count() > MAX_CHAT_CHARS {
            let tail: String = c.chars().rev().take(MAX_CHAT_CHARS).collect::<Vec<_>>().into_iter().rev().collect();
            format!("[earlier messages left out]\n{tail}")
        } else {
            c
        }
    });
    Ok(Pack { project: p.doc().project.name.clone(), timeline: p.projection(), history, presets: presets(p), chat })
}

/// The pack as text: the request sent to the assistant, and what `--pack-out` saves.
pub fn pack_text(pack: &Pack) -> String {
    let mut s = format!("# Context pack: {}\n\n## Final timeline (timeline.tsx)\n\n```tsx\n", pack.project);
    s.push_str(pack.timeline.trim_end());
    s.push_str("\n```\n\n## Edit history by actor\n\n");
    if pack.history.is_empty() {
        s.push_str("(no revisions)\n");
    }
    for (who, lines) in &pack.history {
        s.push_str(&format!("### {who}\n\n"));
        for l in lines {
            s.push_str(&format!("- {l}\n"));
        }
        s.push('\n');
    }
    s.push_str("## Presets and looks in use\n\n");
    if pack.presets.0.is_empty() {
        s.push_str("(none)\n");
    }
    for (kind, values) in &pack.presets.0 {
        let list: Vec<String> = values.iter().map(|(v, n)| format!("{v} ({n})")).collect();
        s.push_str(&format!("- {kind}: {}\n", list.join(", ")));
    }
    if let Some(chat) = &pack.chat {
        s.push_str("\n## Chat with the assistant\n\n");
        s.push_str(chat.trim_end());
        s.push('\n');
    }
    s
}

/// Our drafting instructions (the system prompt of the drafting request).
pub const INSTRUCTIONS: &str = r#"You turn one finished video edit into a reusable playbook ("skill") for agents that edit videos in this editor through its tools (print_timeline, apply_timeline, edit_timeline, look, verify_timeline and others).

You get a context pack: the final timeline as printed text, the edit history grouped by who made each change, the presets and looks in use, and sometimes the chat that led to the edit.

Write one Markdown file that answers two questions, each in its own section:
1. Rebuild: how an agent rebuilds this edit from the same media with no prior context. Give the steps in order, with the concrete values used (times, durations, preset ids, levels, colours), and the checks to run.
2. Reuse: how an agent applies the same style to new media. Separate what is style (keep it) from what depends on this footage (decide it again), and say how to decide.

Rules:
- Start with front matter: `name` (at most 80 characters), `description` (one or two sentences: what the skill does and when to use it, at most 300 characters), and `source: drafted`.
- Use only tool names and preset ids that appear in the pack or that the editor documents. Do not invent tools.
- Prefer short numbered steps. Each step is one action an agent can do.
- Do not copy the whole timeline. Quote only the lines that carry a value worth repeating.
- Write plain English. No marketing words.
- Output the file only, with no text before or after it."#;

/// The user message of the drafting request.
pub fn request(pack: &Pack) -> String {
    format!("{}\n\nWrite the skill file now.", pack_text(pack))
}

/// Cleans the assistant's draft: drops a surrounding code fence, makes sure the front matter says `source: drafted`,
/// and checks it is a valid skill. Refused with `draft.invalid` when it is not.
pub fn finish(draft: &str) -> Result<String, Fail> {
    let mut t = draft.trim().to_string();
    if t.starts_with("```") {
        let inner = t.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
        t = inner.trim_end().strip_suffix("```").unwrap_or(inner).trim().to_string();
    }
    let fm: FrontMatter = parse_front_matter(&t)
        .map_err(|why| Fail::new("draft.invalid", format!("the draft is not a skill file: {why}")))?;
    if fm.source.is_none()
        && let Some((front, body)) = split_front_matter(&t)
    {
        t = format!(
            "---\n{}source: drafted\n---\n{body}",
            if front.ends_with('\n') { front.to_string() } else { format!("{front}\n") }
        );
    }
    t.push('\n');
    validate_text(&t).map_err(|why| Fail::new("draft.invalid", format!("the draft is not a usable skill: {why}")))?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ove_core::ops::Op;
    use ove_core::revision::{Actor, ActorKind, Source};
    use serde_json::json;

    use super::*;

    fn project() -> (tempfile::TempDir, Project) {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("Interview").unwrap();
        let ops: Vec<Op> = serde_json::from_value(json!([
            {"op": "addTrack", "track": {"id": "t_cap", "name": "Captions", "order": 0, "type": "caption"}},
            {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 1, "type": "video"}},
            {"op": "addItem", "item": {"id": "c1", "trackId": "t_cap", "type": "caption", "startTime": 0, "endTime": 2, "captionText": "Hello", "captionPresetId": "word-pop"}},
            {"op": "addItem", "item": {"id": "t1", "trackId": "t_main", "type": "text", "startTime": 0, "endTime": 2, "textContent": "Hi", "transitionIn": {"type": "dissolve", "duration": 0.5}}}
        ]))
        .unwrap();
        p.apply(ops, Source::Gui, Some("starter tracks".into())).unwrap();
        let ops: Vec<Op> = serde_json::from_value(json!([
            {"op": "updateItem", "id": "t1", "set": {"colorGrading": {"exposure": 0.3, "contrast": 0}}}
        ]))
        .unwrap();
        let actor = Actor { kind: ActorKind::Assistant, session: Some("c1".into()), turn: None, client: None };
        p.apply_as(ops, Source::Agent, Some("brighter title".into()), Some(actor)).unwrap();
        (d, p)
    }

    #[test]
    fn the_pack_holds_the_timeline_history_by_actor_and_presets() {
        let (_d, p) = project();
        let k = pack(&p, None).unwrap();
        assert!(k.timeline.contains("<timeline"));
        assert_eq!(k.history["assistant"].len(), 1);
        assert!(k.history["assistant"][0].ends_with(" (agent): brighter title"), "{:?}", k.history);
        assert!(k.history.values().flatten().any(|l| l.contains("starter tracks")));
        assert_eq!(k.presets.0["caption style"]["word-pop"], 1);
        assert_eq!(k.presets.0["transition"]["dissolve"], 1);
        assert_eq!(k.presets.0["colour adjustment"]["exposure 0.3"], 1);
        let text = pack_text(&k);
        assert!(text.contains("## Final timeline") && text.contains("### assistant") && text.contains("word-pop (1)"));
        assert!(!text.contains("## Chat"));
        let with_chat = pack_text(&pack(&p, Some("User: make the title brighter".into())).unwrap());
        assert!(with_chat.contains("## Chat with the assistant\n\nUser: make the title brighter"));
    }

    #[test]
    fn drafts_are_cleaned_and_checked() {
        let fenced = "```markdown\n---\nname: Bright titles\ndescription: Make titles stand out.\n---\n\n## Rebuild\n1. Step.\n```";
        let t = finish(fenced).unwrap();
        assert!(
            t.starts_with("---\nname: Bright titles\ndescription: Make titles stand out.\nsource: drafted\n---\n"),
            "{t}"
        );
        assert!(t.contains("## Rebuild"));
        assert_eq!(finish("No front matter here").unwrap_err().code, "draft.invalid");
        assert_eq!(finish("---\nname: X\n---\nbody").unwrap_err().code, "draft.invalid");
        let kept = finish("---\nname: X\ndescription: Y\nsource: drafted\n---\nbody").unwrap();
        assert_eq!(kept.matches("source:").count(), 1);
    }

    #[test]
    fn the_instructions_name_only_real_tools() {
        assert_eq!(crate::skills::unknown_tool_names(INSTRUCTIONS), Vec::<String>::new());
    }
}
