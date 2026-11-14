//! Chat logs in the project bundle (docs/spec/document-model.md, Chat log records): `agent/chats.json` is the index
//! and `agent/chats/<id>.jsonl` holds one record per line, appended only. A conversation is resumed by rebuilding the
//! API messages from its records, with every assistant block replayed exactly as received.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::Usage;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Record {
    #[serde(rename_all = "camelCase")]
    User { id: String, text: String, time: i64 },
    #[serde(rename_all = "camelCase")]
    Session { session_id: String, model: String },
    /// Content blocks exactly as the API returned them (text, tool_use, thinking, fallback, …).
    #[serde(rename_all = "camelCase")]
    Assistant { message_id: String, model: String, blocks: Vec<Value> },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        tool_use_id: String,
        text: String,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(rename_all = "camelCase")]
    Question { id: String, questions: Vec<QuestionItem> },
    /// The end of a turn: its cost and token usage (`thread_usage` is the turn's own), and the note it stopped
    /// with (a limit that was reached).
    #[serde(rename_all = "camelCase")]
    Result {
        is_error: bool,
        duration_ms: u64,
        cost_usd: f64,
        text: String,
        thread_usage: Usage,
        /// When the turn ended, ms since the epoch (0 in logs written before it was recorded).
        #[serde(default)]
        time: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Project revisions the turn that began with user record `turn` made (one record per tool call that changed
    /// the project).
    #[serde(rename_all = "camelCase")]
    Revisions { turn: String, revs: Vec<i64> },
    /// "Undo this turn" reverted the revisions of `turn` as project revision `rev`.
    #[serde(rename_all = "camelCase")]
    TurnUndone { turn: String, rev: i64 },
    /// The conversation before the `first_kept`-th user record (0-based) is sent as this summary instead of in
    /// full. The records stay in the log, so the chat still shows everything.
    #[serde(rename_all = "camelCase")]
    Summary { first_kept: usize, text: String, time: i64 },
}

/// Tokens and estimated cost of some turns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub usage: Usage,
    pub cost_usd: f64,
    pub turns: u64,
}

impl Spend {
    pub fn add(&mut self, o: &Spend) {
        self.usage.add(&o.usage);
        self.cost_usd += o.cost_usd;
        self.turns += o.turns;
    }
}

/// The spend of the turns in `records` that ended at or after `since_ms`.
pub fn spend_of(records: &[Record], since_ms: i64) -> Spend {
    let mut s = Spend::default();
    for r in records {
        if let Record::Result { cost_usd, thread_usage, time, .. } = r
            && *time >= since_ms
        {
            s.usage.add(thread_usage);
            s.cost_usd += cost_usd;
            s.turns += 1;
        }
    }
    s
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionItem {
    pub header: String,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub model: String,
}

/// The chats of one project bundle.
#[derive(Debug, Clone)]
pub struct ChatStore {
    dir: PathBuf,
}

fn now_ms() -> i64 {
    ove_core::Clock::now_ms(&ove_core::SystemClock)
}

impl ChatStore {
    pub fn for_bundle(bundle: &Path) -> Self {
        ChatStore { dir: bundle.join("agent") }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("chats.json")
    }

    /// The log file of chat `id`; refused unless the id is a plain name ([`ove_guard::is_safe_id`]), since ids come
    /// from the bundle's index and from callers.
    fn log_path(&self, id: &str) -> std::io::Result<PathBuf> {
        if !ove_guard::is_safe_id(id) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("invalid chat id {id:?}")));
        }
        Ok(self.dir.join("chats").join(format!("{id}.jsonl")))
    }

    pub fn list(&self) -> Vec<ChatSummary> {
        let mut v: Vec<ChatSummary> =
            std::fs::read(self.index_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        v.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        v
    }

    fn write_index(&self, v: &[ChatSummary]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(".chats.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
        std::fs::rename(tmp, self.index_path())
    }

    /// Creates a chat titled after its first message.
    pub fn create(&self, first_text: &str, model: &str) -> std::io::Result<ChatSummary> {
        let t = now_ms();
        let mut title: String = first_text.trim().lines().next().unwrap_or("New chat").chars().take(60).collect();
        if title.is_empty() {
            title = "New chat".into();
        }
        let s = ChatSummary { id: format!("c_{t:x}"), title, created_at: t, updated_at: t, model: model.into() };
        let mut v = self.list();
        v.push(s.clone());
        self.write_index(&v)?;
        self.append(&s.id, &Record::Session { session_id: s.id.clone(), model: model.into() })?;
        Ok(s)
    }

    pub fn touch(&self, id: &str, model: &str) {
        let mut v = self.list();
        if let Some(s) = v.iter_mut().find(|s| s.id == id) {
            s.updated_at = now_ms();
            s.model = model.into();
        }
        let _ = self.write_index(&v);
    }

    pub fn append(&self, id: &str, r: &Record) -> std::io::Result<()> {
        let p = self.log_path(id)?;
        std::fs::create_dir_all(self.dir.join("chats"))?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
        let mut line = serde_json::to_vec(r)?;
        line.push(b'\n');
        f.write_all(&line)
    }

    /// The spend of chat `id` since `since_ms` (0: all of it).
    pub fn spend(&self, id: &str, since_ms: i64) -> Spend {
        spend_of(&self.records(id), since_ms)
    }

    /// The spend of every chat of the project since `since_ms`.
    pub fn project_spend(&self, since_ms: i64) -> Spend {
        let mut s = Spend::default();
        for c in self.list() {
            s.add(&self.spend(&c.id, since_ms));
        }
        s
    }

    /// Records of a chat; a torn last line (crash mid-write) is skipped. An invalid id has none.
    pub fn records(&self, id: &str) -> Vec<Record> {
        let Ok(path) = self.log_path(id) else { return Vec::new() };
        std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }
}

/// How a summary of earlier turns opens when it is sent.
pub const SUMMARY_PREFIX: &str =
    "[Summary of the earlier part of this conversation, written to keep it within the model's limits]\n";

/// The latest summary in `records` and the index of the first record sent after it (the user record it keeps
/// from). `(None, 0)` when there is none.
pub fn summary_cut(records: &[Record]) -> (Option<&str>, usize) {
    let Some((text, first_kept)) = records.iter().rev().find_map(|r| match r {
        Record::Summary { text, first_kept, .. } => Some((text.as_str(), *first_kept)),
        _ => None,
    }) else {
        return (None, 0);
    };
    match user_indices(records).get(first_kept) {
        Some(i) => (Some(text), *i),
        None => (None, 0),
    }
}

/// Indices of the user records (the start of each turn).
pub fn user_indices(records: &[Record]) -> Vec<usize> {
    records.iter().enumerate().filter(|(_, r)| matches!(r, Record::User { .. })).map(|(i, _)| i).collect()
}

/// Rough input-token estimate of some JSON (about four bytes per token).
pub fn estimate_tokens(v: &Value) -> u64 {
    (v.to_string().len() as u64).div_ceil(4)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(" […]");
    out
}

/// A plain-text transcript of records for summarising: messages in full (up to a length), tool calls and results
/// shortened, thinking left out. When longer than `max_chars`, the oldest part is dropped.
pub fn transcript(records: &[Record], max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for r in records {
        match r {
            Record::User { text, .. } => lines.push(format!("User: {}", clip(text, 4000))),
            Record::Assistant { blocks, .. } => {
                for b in blocks {
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or_default();
                            if !t.trim().is_empty() {
                                lines.push(format!("Assistant: {}", clip(t, 4000)));
                            }
                        }
                        Some("tool_use") => lines.push(format!(
                            "Assistant used {}: {}",
                            b["name"].as_str().unwrap_or("a tool"),
                            clip(&b["input"].to_string(), 300)
                        )),
                        _ => {}
                    }
                }
            }
            Record::ToolResult { text, is_error, .. } => {
                lines.push(format!("{}: {}", if *is_error { "Tool error" } else { "Tool result" }, clip(text, 300)))
            }
            Record::Summary { text, .. } => lines.push(format!("Earlier summary: {text}")),
            _ => {}
        }
    }
    let mut out = lines.join("\n");
    if out.len() > max_chars {
        let mut cut = out.len() - max_chars;
        while !out.is_char_boundary(cut) {
            cut += 1;
        }
        out = format!("[…the oldest part is left out…]\n{}", &out[cut..]);
    }
    out
}

/// Checks that `msgs` is a valid Messages API history: it starts with the user, roles alternate, every tool call is
/// answered in the next message and every tool result answers a call in the message before it.
pub fn validate_messages(msgs: &[Value]) -> Result<(), String> {
    let mut expected: Vec<String> = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        let role = m["role"].as_str().unwrap_or_default();
        let want = if i % 2 == 0 { "user" } else { "assistant" };
        if role != want {
            return Err(format!("message {i} is from {role:?}, expected {want}"));
        }
        let blocks = m["content"].as_array().ok_or(format!("message {i} has no content blocks"))?;
        let ids = |kind: &str, key: &str| -> Vec<String> {
            blocks.iter().filter(|b| b["type"] == kind).filter_map(|b| b[key].as_str().map(str::to_string)).collect()
        };
        if role == "user" {
            let mut answered = ids("tool_result", "tool_use_id");
            answered.sort();
            let mut want = std::mem::take(&mut expected);
            want.sort();
            if answered != want {
                return Err(format!("message {i} answers tool calls {answered:?}, expected {want:?}"));
            }
        } else {
            expected = ids("tool_use", "id");
        }
    }
    if !expected.is_empty() {
        return Err(format!("tool calls {expected:?} are never answered"));
    }
    Ok(())
}

/// Rebuilds API `messages` from records. Tool results are grouped into one user message, consecutive user content
/// is merged, and tool calls left unanswered (a stopped turn) get an error result so the history stays valid. When
/// the log holds a summary, the turns it covers are sent as that summary.
pub fn messages(records: &[Record]) -> Vec<Value> {
    let (summary, start) = summary_cut(records);
    let mut msgs: Vec<Value> = Vec::new();
    let mut open_calls: Vec<String> = Vec::new();
    fn push_user(msgs: &mut Vec<Value>, block: Value) {
        if let Some(last) = msgs.last_mut()
            && last["role"] == "user"
            && let Some(content) = last["content"].as_array_mut()
        {
            content.push(block);
            return;
        }
        msgs.push(json!({"role": "user", "content": [block]}));
    }
    let close_open = |msgs: &mut Vec<Value>, open: &mut Vec<String>| {
        for id in open.drain(..) {
            push_user(
                msgs,
                json!({"type": "tool_result", "tool_use_id": id, "content": "Stopped by the user before this ran.", "is_error": true}),
            );
        }
    };
    if let Some(text) = summary {
        push_user(&mut msgs, json!({"type": "text", "text": format!("{SUMMARY_PREFIX}{text}")}));
    }
    for r in &records[start..] {
        match r {
            Record::User { text, .. } => {
                close_open(&mut msgs, &mut open_calls);
                push_user(&mut msgs, json!({"type": "text", "text": text}));
            }
            Record::Assistant { blocks, .. } => {
                close_open(&mut msgs, &mut open_calls);
                open_calls = blocks
                    .iter()
                    .filter(|b| b["type"] == "tool_use")
                    .filter_map(|b| b["id"].as_str().map(str::to_string))
                    .collect();
                msgs.push(json!({"role": "assistant", "content": blocks}));
            }
            Record::ToolResult { tool_use_id, text, is_error } => {
                open_calls.retain(|i| i != tool_use_id);
                let mut b = json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": text});
                if *is_error {
                    b["is_error"] = json!(true);
                }
                push_user(&mut msgs, b);
            }
            _ => {}
        }
    }
    msgs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rebuild() {
        let d = tempfile::tempdir().unwrap();
        let store = ChatStore::for_bundle(d.path());
        let c = store.create("Make a 30 second teaser\nwith music", "claude-sonnet-5-5").unwrap();
        assert_eq!(c.title, "Make a 30 second teaser");
        store.append(&c.id, &Record::User { id: "u1".into(), text: "hi".into(), time: 1 }).unwrap();
        let blocks = vec![
            json!({"type": "thinking", "thinking": "", "signature": "s"}),
            json!({"type": "text", "text": "Let me look."}),
            json!({"type": "tool_use", "id": "t1", "name": "print_timeline", "input": {}}),
            json!({"type": "tool_use", "id": "t2", "name": "list_media", "input": {}}),
        ];
        store
            .append(&c.id, &Record::Assistant { message_id: "m1".into(), model: "x".into(), blocks: blocks.clone() })
            .unwrap();
        store
            .append(&c.id, &Record::ToolResult { tool_use_id: "t1".into(), text: "tsx".into(), is_error: false })
            .unwrap();
        // t2 was never answered (stopped); then the user writes again.
        store.append(&c.id, &Record::User { id: "u2".into(), text: "continue".into(), time: 2 }).unwrap();
        let recs = store.records(&c.id);
        assert_eq!(recs.len(), 5);
        let m = messages(&recs);
        assert_eq!(m.len(), 3);
        assert_eq!(m[1]["content"], json!(blocks));
        let last = m[2]["content"].as_array().unwrap();
        assert_eq!(last.len(), 3);
        assert_eq!(last[0]["tool_use_id"], "t1");
        assert_eq!(last[1]["tool_use_id"], "t2");
        assert_eq!(last[1]["is_error"], true);
        assert_eq!(last[2]["text"], "continue");
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn chat_ids_cannot_escape_the_chats_folder() {
        let d = tempfile::tempdir().unwrap();
        let bundle = d.path().join("p.ove");
        let store = ChatStore::for_bundle(&bundle);
        let r = Record::User { id: "u".into(), text: "x".into(), time: 0 };
        for id in ["../../escape", "..", "a/b", "a\\b", "/tmp/abs", "", "con"] {
            let e = store.append(id, &r).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{id:?}");
            assert!(store.records(id).is_empty());
        }
        assert!(!d.path().join("escape.jsonl").exists());
        assert!(!bundle.join("escape.jsonl").exists());
        // An index entry pointing outside (a bundle made elsewhere) reads as an empty chat.
        std::fs::create_dir_all(bundle.join("agent")).unwrap();
        std::fs::write(bundle.join("secret.jsonl"), "{\"type\":\"user\",\"id\":\"u\",\"text\":\"s\",\"time\":0}\n")
            .unwrap();
        assert!(store.records("../../secret").is_empty());
    }

    #[test]
    fn a_summary_replaces_the_turns_before_the_one_it_keeps() {
        let user = |i: i64| Record::User { id: format!("u{i}"), text: format!("turn {i}"), time: i };
        let call = |id: &str| Record::Assistant {
            message_id: "m".into(),
            model: "x".into(),
            blocks: vec![json!({"type": "tool_use", "id": id, "name": "print_timeline", "input": {}})],
        };
        let result = |id: &str| Record::ToolResult { tool_use_id: id.into(), text: "t".into(), is_error: false };
        let answer = Record::Assistant {
            message_id: "m".into(),
            model: "x".into(),
            blocks: vec![json!({"type": "text", "text": "done"})],
        };
        let mut recs = vec![
            user(0),
            call("a"),
            result("a"),
            answer.clone(),
            user(1),
            call("b"),
            result("b"),
            answer.clone(),
            user(2),
        ];
        let full = messages(&recs);
        validate_messages(&full).unwrap();
        assert_eq!(full.len(), 9);
        recs.push(Record::Summary { first_kept: 2, text: "Earlier: two turns.".into(), time: 5 });
        let m = messages(&recs);
        validate_messages(&m).unwrap();
        assert_eq!(m.len(), 1);
        let blocks = m[0]["content"].as_array().unwrap();
        assert!(blocks[0]["text"].as_str().unwrap().starts_with(SUMMARY_PREFIX));
        assert_eq!(blocks[1]["text"], "turn 2");
        // A summary pointing past the end is ignored.
        recs.push(Record::Summary { first_kept: 9, text: "bad".into(), time: 6 });
        assert_eq!(messages(&recs).len(), 9);
        // Transcripts shorten tool output and keep the newest part.
        let t = transcript(&recs[..4], 10_000);
        assert!(t.contains("User: turn 0") && t.contains("Assistant used print_timeline"));
        assert!(transcript(&recs, 40).starts_with("[…the oldest part is left out…]"));
    }

    #[test]
    fn validation_catches_split_tool_pairs() {
        let call = json!({"role": "assistant", "content": [{"type": "tool_use", "id": "a", "name": "x", "input": {}}]});
        let user = json!({"role": "user", "content": [{"type": "text", "text": "hi"}]});
        assert!(validate_messages(std::slice::from_ref(&call)).is_err(), "must start with the user");
        assert!(validate_messages(&[user.clone(), call.clone()]).is_err(), "unanswered call");
        assert!(validate_messages(&[user.clone(), call, user]).is_err(), "answer missing");
    }

    #[test]
    fn spend_adds_up_results_since_a_time() {
        let d = tempfile::tempdir().unwrap();
        let store = ChatStore::for_bundle(d.path());
        let result = |cost: f64, time: i64| Record::Result {
            is_error: false,
            duration_ms: 1,
            cost_usd: cost,
            text: String::new(),
            thread_usage: Usage { input_tokens: 10, output_tokens: 2, ..Default::default() },
            time,
            note: None,
        };
        let a = store.create("a", "m").unwrap();
        store.append(&a.id, &result(0.5, 100)).unwrap();
        store.append(&a.id, &result(0.25, 300)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = store.create("b", "m").unwrap();
        store.append(&b.id, &result(1.0, 400)).unwrap();
        assert_eq!(store.spend(&a.id, 0).turns, 2);
        assert!((store.spend(&a.id, 0).cost_usd - 0.75).abs() < 1e-12);
        let p = store.project_spend(200);
        assert_eq!(p.turns, 2);
        assert!((p.cost_usd - 1.25).abs() < 1e-12);
        assert_eq!(p.usage.input_tokens, 20);
        // Results logged before times were recorded count in all-time totals only.
        let old: Record = serde_json::from_str(
            r#"{"type":"result","isError":false,"durationMs":1,"costUsd":2.0,"text":"","threadUsage":{"inputTokens":1,"outputTokens":1,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}"#,
        )
        .unwrap();
        assert_eq!(spend_of(std::slice::from_ref(&old), 1).turns, 0);
        assert_eq!(spend_of(&[old], 0).turns, 1);
    }

    #[test]
    fn torn_line_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let store = ChatStore::for_bundle(d.path());
        let c = store.create("x", "m").unwrap();
        store.append(&c.id, &Record::User { id: "u".into(), text: "a".into(), time: 0 }).unwrap();
        let p = d.path().join("agent/chats").join(format!("{}.jsonl", c.id));
        let mut s = std::fs::read_to_string(&p).unwrap();
        s.push_str("{\"type\":\"user\",\"te");
        std::fs::write(&p, s).unwrap();
        assert_eq!(store.records(&c.id).len(), 2);
    }
}
