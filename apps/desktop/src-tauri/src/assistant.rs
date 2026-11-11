//! The assistant in the app: API key in the OS credential store, model choice, one running turn at a time over the
//! open project, chat history per project, and question cards. Edits go through the document pipeline as `agent`
//! revisions (one undo step each) and refresh the editor like any other edit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ove_agent::tools::{self, ApplyRequest, ToolHost, label};
use ove_agent::{AgentEvent, ChatStore, Client, KeyStore, OsKeyStore, Record, TurnConfig, run_turn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectChanged, view};

struct Running {
    cancel: Arc<AtomicBool>,
}

/// Assistant state shared by the commands and the turn thread.
pub struct Assistant {
    keys: Box<dyn KeyStore>,
    model: Mutex<String>,
    running: Mutex<Option<Running>>,
    answers: Mutex<HashMap<String, Sender<String>>>,
    settings_path: Option<PathBuf>,
}

impl Assistant {
    pub fn new(settings_dir: Option<PathBuf>) -> Self {
        Self::with_keys(Box::new(OsKeyStore), settings_dir)
    }

    pub fn with_keys(keys: Box<dyn KeyStore>, settings_dir: Option<PathBuf>) -> Self {
        let settings_path = settings_dir.map(|d| d.join("assistant.json"));
        let model = settings_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v["model"].as_str().map(str::to_string))
            .filter(|m| ove_agent::MODELS.iter().any(|x| x.id == m))
            .unwrap_or_else(|| ove_agent::MODELS[0].id.to_string());
        Assistant {
            keys,
            model: Mutex::new(model),
            running: Mutex::new(None),
            answers: Mutex::new(HashMap::new()),
            settings_path,
        }
    }
}

fn lock<'a, T>(m: &'a Mutex<T>) -> AppResult<std::sync::MutexGuard<'a, T>> {
    m.lock().map_err(|_| AppError::new(ErrorCode::Internal, "assistant state is unavailable"))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub has_key: bool,
    /// "keychain" or "environment".
    pub key_source: Option<String>,
    pub model: String,
    pub models: Vec<ModelOption>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummaryView {
    pub id: String,
    pub title: String,
    pub updated_at: f64,
}

/// One line of the chat as the UI shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatItem {
    /// user, assistant, tool, question or note.
    pub role: String,
    pub text: String,
    /// Tool call / question id.
    pub id: Option<String>,
    pub ok: Option<bool>,
    pub options: Vec<String>,
}

/// Live assistant activity.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentUiEvent {
    #[serde(rename_all = "camelCase")]
    TurnStarted {
        chat_id: String,
    },
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        label: String,
    },
    ToolResult {
        id: String,
        ok: bool,
        summary: String,
    },
    Question {
        id: String,
        question: String,
        options: Vec<String>,
    },
    Retrying {
        message: String,
        seconds: f64,
    },
    #[serde(rename_all = "camelCase")]
    TurnDone {
        chat_id: String,
        stopped: bool,
        duration_ms: f64,
        input_tokens: f64,
        output_tokens: f64,
        cache_read_tokens: f64,
        cost_usd: f64,
        note: Option<String>,
    },
    Error {
        code: String,
        message: String,
    },
    /// The assistant asked to show a timeline time.
    Seek {
        seconds: f64,
    },
}

impl From<AgentEvent> for AgentUiEvent {
    fn from(e: AgentEvent) -> Self {
        match e {
            AgentEvent::TurnStarted { chat_id } => AgentUiEvent::TurnStarted { chat_id },
            AgentEvent::TextDelta { text } => AgentUiEvent::TextDelta { text },
            AgentEvent::ThinkingDelta { text } => AgentUiEvent::ThinkingDelta { text },
            AgentEvent::ToolCall { id, name, label } => AgentUiEvent::ToolCall { id, name, label },
            AgentEvent::ToolResult { id, ok, summary } => AgentUiEvent::ToolResult { id, ok, summary },
            AgentEvent::Question { id, question, options } => AgentUiEvent::Question { id, question, options },
            AgentEvent::Retrying { message, seconds } => AgentUiEvent::Retrying { message, seconds },
            AgentEvent::TurnDone { chat_id, stopped, duration_ms, usage, cost_usd, note } => AgentUiEvent::TurnDone {
                chat_id,
                stopped,
                duration_ms: duration_ms as f64,
                input_tokens: (usage.input_tokens + usage.cache_creation_input_tokens) as f64,
                output_tokens: usage.output_tokens as f64,
                cache_read_tokens: usage.cache_read_input_tokens as f64,
                cost_usd,
                note,
            },
            AgentEvent::Error { code, message } => AgentUiEvent::Error { code, message },
        }
    }
}

/// Tools over the project open in the editor.
struct DesktopHost {
    app: AppHandle,
    cancel: Arc<AtomicBool>,
}

impl DesktopHost {
    fn with_project<T>(
        &self,
        f: impl FnOnce(&mut ove_store::Project) -> Result<T, String>,
        changed: bool,
    ) -> Result<T, String> {
        let state = self.app.state::<AppState>();
        let mut g = state.projects.open.lock().map_err(|_| "project state unavailable".to_string())?;
        let p = g.as_mut().ok_or("No project is open in the editor anymore.")?;
        let out = f(p)?;
        if changed {
            let _ = ProjectChanged(Some(view(p))).emit(&self.app);
        }
        Ok(out)
    }
}

impl ToolHost for DesktopHost {
    fn print_timeline(&mut self, from: Option<usize>, lines: Option<usize>) -> Result<String, String> {
        self.with_project(|p| Ok(tools::print(p, from, lines)), false)
    }
    fn apply_timeline(&mut self, req: ApplyRequest) -> Result<String, String> {
        self.with_project(|p| tools::apply(p, req), true)
    }
    fn verify_timeline(&mut self) -> Result<String, String> {
        self.with_project(|p| Ok(tools::verify(p)), false)
    }
    fn list_media(&mut self) -> Result<String, String> {
        self.with_project(|p| Ok(tools::list_media(p)), false)
    }
    fn undo_last(&mut self) -> Result<String, String> {
        self.with_project(tools::undo_last, true)
    }
    fn import_media(&mut self, paths: Vec<String>) -> Result<String, String> {
        let v = tauri::async_runtime::block_on(crate::projects::import_media(self.app.clone(), paths))
            .map_err(|e| e.message)?;
        Ok(format!("Imported. The project now has {} media files and {} items.", v.media.len(), v.items.len()))
    }
    fn seek_viewer(&mut self, seconds: f64) -> Result<String, String> {
        let _ = AgentUiEvent::Seek { seconds }.emit(&self.app);
        Ok(format!("Showing {seconds:.2}s."))
    }
    fn ask_user(&mut self, question_id: &str, _question: &str, _options: &[String]) -> Result<String, String> {
        let state = self.app.state::<AppState>();
        let (tx, rx) = channel();
        state.assistant.answers.lock().map_err(|_| "unavailable")?.insert(question_id.to_string(), tx);
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err("The user stopped the turn without answering.".into());
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(a) => return Ok(a),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err("The question was dismissed.".into()),
            }
        }
    }
}

fn status(a: &Assistant) -> AppResult<AgentStatus> {
    let key = a.keys.get();
    Ok(AgentStatus {
        has_key: key.is_some(),
        key_source: key.map(|(_, s)| match s {
            ove_agent::KeySource::Keychain => "keychain".into(),
            ove_agent::KeySource::Environment => "environment".into(),
        }),
        model: lock(&a.model)?.clone(),
        models: ove_agent::MODELS.iter().map(|m| ModelOption { id: m.id.into(), label: m.label.into() }).collect(),
        running: lock(&a.running)?.is_some(),
    })
}

fn bundle(state: &AppState) -> AppResult<PathBuf> {
    let g = state.projects.open.lock().map_err(|_| AppError::new(ErrorCode::Internal, "project state unavailable"))?;
    g.as_ref()
        .map(|p| p.bundle().to_path_buf())
        .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open or create a project to chat about it."))
}

#[tauri::command]
#[specta::specta]
pub fn agent_status(state: State<'_, AppState>) -> AppResult<AgentStatus> {
    status(&state.assistant)
}

#[tauri::command]
#[specta::specta]
pub fn agent_set_key(state: State<'_, AppState>, key: String) -> AppResult<AgentStatus> {
    state.assistant.keys.set(&key).map_err(|m| AppError::new(ErrorCode::InvalidArgument, m))?;
    tracing::info!("assistant API key stored in the system credential store");
    status(&state.assistant)
}

#[tauri::command]
#[specta::specta]
pub fn agent_clear_key(state: State<'_, AppState>) -> AppResult<AgentStatus> {
    state.assistant.keys.clear().map_err(|m| AppError::new(ErrorCode::Io, m))?;
    status(&state.assistant)
}

#[tauri::command]
#[specta::specta]
pub fn agent_set_model(state: State<'_, AppState>, model: String) -> AppResult<AgentStatus> {
    if !ove_agent::MODELS.iter().any(|m| m.id == model) {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Unknown model."));
    }
    *lock(&state.assistant.model)? = model.clone();
    if let Some(p) = &state.assistant.settings_path {
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
        let _ = std::fs::write(p, serde_json::json!({"model": model}).to_string());
    }
    status(&state.assistant)
}

#[tauri::command]
#[specta::specta]
pub fn agent_chats(state: State<'_, AppState>) -> AppResult<Vec<ChatSummaryView>> {
    let Ok(b) = bundle(&state) else { return Ok(vec![]) };
    Ok(ChatStore::for_bundle(&b)
        .list()
        .into_iter()
        .map(|c| ChatSummaryView { id: c.id, title: c.title, updated_at: c.updated_at as f64 })
        .collect())
}

/// A chat's records as display items.
pub fn chat_items(records: &[Record]) -> Vec<ChatItem> {
    let item = |role: &str, text: String| ChatItem { role: role.into(), text, id: None, ok: None, options: vec![] };
    let mut out: Vec<ChatItem> = Vec::new();
    for r in records {
        match r {
            Record::User { text, .. } => out.push(item("user", text.clone())),
            Record::Assistant { blocks, .. } => {
                for b in blocks {
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(item("assistant", t.to_string()));
                            }
                        }
                        Some("tool_use") if b["name"] != "ask_user" => {
                            let name = b["name"].as_str().unwrap_or_default();
                            out.push(ChatItem {
                                id: b["id"].as_str().map(str::to_string),
                                ..item("tool", label(name, &b["input"]))
                            });
                        }
                        _ => {}
                    }
                }
            }
            Record::ToolResult { tool_use_id, text, is_error } => {
                if let Some(it) = out.iter_mut().rev().find(|i| i.id.as_deref() == Some(tool_use_id)) {
                    if it.role == "question" {
                        it.text = format!("{}\n→ {}", it.text, text);
                    }
                    it.ok = Some(!is_error);
                }
            }
            Record::Question { id, questions } => {
                if let Some(q) = questions.first() {
                    out.push(ChatItem {
                        id: Some(id.clone()),
                        options: q.options.iter().map(|o| o.label.clone()).collect(),
                        ..item("question", q.question.clone())
                    });
                }
            }
            Record::Result { is_error: true, text, .. } => out.push(item("note", text.clone())),
            _ => {}
        }
    }
    out
}

#[tauri::command]
#[specta::specta]
pub fn agent_chat(state: State<'_, AppState>, id: String) -> AppResult<Vec<ChatItem>> {
    Ok(chat_items(&ChatStore::for_bundle(&bundle(&state)?).records(&id)))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub chat_id: String,
}

/// Starts a turn in a chat (a new chat when `chat_id` is empty). Progress arrives as `agentUiEvent`.
#[tauri::command]
#[specta::specta]
pub fn agent_send(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: Option<String>,
    text: String,
) -> AppResult<SendResult> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Type a message first."));
    }
    let a = &state.assistant;
    let Some((key, _)) = a.keys.get() else {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Add your Anthropic API key to use the assistant."));
    };
    let bundle = bundle(&state)?;
    let model = lock(&a.model)?.clone();
    let store = ChatStore::for_bundle(&bundle);
    let chat_id = match chat_id.filter(|c| !c.is_empty()) {
        Some(c) => c,
        None => {
            store
                .create(&text, &model)
                .map_err(|e| AppError::new(ErrorCode::Io, format!("Couldn't start the chat: {e}")))?
                .id
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut running = lock(&a.running)?;
        if running.is_some() {
            return Err(AppError::new(ErrorCode::InvalidArgument, "The assistant is still working. Stop it or wait."));
        }
        *running = Some(Running { cancel: cancel.clone() });
    }
    let (app2, chat2) = (app.clone(), chat_id.clone());
    std::thread::Builder::new()
        .name("assistant-turn".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let client = Client::new(key);
                let mut host = DesktopHost { app: app2.clone(), cancel: cancel.clone() };
                let cfg = TurnConfig { model, ..TurnConfig::default() };
                let _ = run_turn(&client, &store, &chat2, &text, &mut host, &cfg, &cancel, &mut |e| {
                    let _ = AgentUiEvent::from(e).emit(&app2);
                });
            }));
            if result.is_err() {
                tracing::error!("the assistant turn panicked");
                let _ = AgentUiEvent::Error {
                    code: "internal".into(),
                    message: "The assistant stopped unexpectedly. Details are in the log.".into(),
                }
                .emit(&app2);
                let _ = AgentUiEvent::TurnDone {
                    chat_id: chat2,
                    stopped: true,
                    duration_ms: 0.0,
                    input_tokens: 0.0,
                    output_tokens: 0.0,
                    cache_read_tokens: 0.0,
                    cost_usd: 0.0,
                    note: None,
                }
                .emit(&app2);
            }
            let st = app2.state::<AppState>();
            if let Ok(mut r) = st.assistant.running.lock() {
                *r = None;
            }
            if let Ok(mut q) = st.assistant.answers.lock() {
                q.clear();
            }
        })
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(SendResult { chat_id })
}

#[tauri::command]
#[specta::specta]
pub fn agent_stop(state: State<'_, AppState>) -> AppResult<()> {
    if let Some(r) = lock(&state.assistant.running)?.as_ref() {
        r.cancel.store(true, Ordering::SeqCst);
    }
    lock(&state.assistant.answers)?.clear();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn agent_answer(state: State<'_, AppState>, question_id: String, answer: String) -> AppResult<()> {
    let tx = lock(&state.assistant.answers)?.remove(&question_id);
    match tx {
        Some(tx) => tx
            .send(answer)
            .map_err(|_| AppError::new(ErrorCode::InvalidArgument, "That question is no longer waiting.")),
        None => Err(AppError::new(ErrorCode::InvalidArgument, "That question is no longer waiting.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::Op;
    use ove_core::revision::Source;
    use serde_json::json;

    #[test]
    fn tools_over_a_temp_project_create_an_agent_revision() {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("T").unwrap();
        p.apply(
            vec![Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 0, "type": "video"}) }],
            Source::Gui,
            None,
        )
        .unwrap();
        let text = tools::print(&p, None, None);
        let anchor = text.lines().find(|l| l.contains("<track ")).unwrap().to_string() + "\n";
        let edit = tools::TextEdit {
            old_text: anchor.clone(),
            new_text: format!("{anchor}      <text start={{0}} end={{2}} textContent=\"Hi\" />\n"),
        };
        let rev = p.rev();
        let out =
            tools::apply(&mut p, ApplyRequest { rev: Some(rev), edits: Some(vec![edit]), source: None, prefer: None })
                .unwrap();
        assert!(out.contains("\"changed\":true"), "{out}");
        assert_eq!(p.latest_record().unwrap().unwrap().source, Source::Agent);
        assert!(p.session().can_undo());
        let items = chat_items(&[
            Record::User { id: "u".into(), text: "add a title".into(), time: 0 },
            Record::Assistant {
                message_id: "m".into(),
                model: "x".into(),
                blocks: vec![
                    json!({"type": "text", "text": "On it."}),
                    json!({"type": "tool_use", "id": "t1", "name": "apply_timeline", "input": {"edits": [{}]}}),
                ],
            },
            Record::ToolResult { tool_use_id: "t1".into(), text: "{}".into(), is_error: false },
        ]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[2].role, "tool");
        assert_eq!(items[2].ok, Some(true));
        assert!(items[2].text.contains("1 change"));
    }

    #[test]
    fn status_reports_key_and_models() {
        let a = Assistant::with_keys(Box::new(ove_agent::MemoryKeyStore::default()), None);
        let s = status(&a).unwrap();
        assert!(!s.has_key);
        assert_eq!(s.model, "claude-sonnet-5-5");
        assert_eq!(s.models.len(), 3);
        a.keys.set("sk-ant-api03-0123456789abcdef").unwrap();
        assert!(status(&a).unwrap().has_key);
    }
}
