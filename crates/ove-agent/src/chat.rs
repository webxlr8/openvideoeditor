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
    #[serde(rename_all = "camelCase")]
    Result { is_error: bool, duration_ms: u64, cost_usd: f64, text: String, thread_usage: Usage },
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
        std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap())?;
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
        std::fs::create_dir_all(p.parent().unwrap())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
        let mut line = serde_json::to_vec(r).unwrap();
        line.push(b'\n');
        f.write_all(&line)
    }

    /// Records of a chat; a torn last line (crash mid-write) is skipped. An invalid id has none.
    pub fn records(&self, id: &str) -> Vec<Record> {
        let Ok(path) = self.log_path(id) else { return Vec::new() };
        std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }
}

/// Rebuilds API `messages` from records. Tool results are grouped into one user message, consecutive user content
/// is merged, and tool calls left unanswered (a stopped turn) get an error result so the history stays valid.
pub fn messages(records: &[Record]) -> Vec<Value> {
    let mut msgs: Vec<Value> = Vec::new();
    let mut open_calls: Vec<String> = Vec::new();
    fn push_user(msgs: &mut Vec<Value>, block: Value) {
        if let Some(last) = msgs.last_mut()
            && last["role"] == "user"
        {
            last["content"].as_array_mut().unwrap().push(block);
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
    for r in records {
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
