//! The assistant in the app: API key in the OS credential store, model choice, one running turn at a time over the
//! open project, chat history per project, and question cards. Edits go through the document pipeline as `agent`
//! revisions (one undo step each) and refresh the editor like any other edit.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ove_agent::tools::{self, ApplyRequest, ToolHost, label};
use ove_agent::{
    AgentEvent, ChatStore, Client, KeyStore, OsKeyStore, ProjectCap, Record, Spend, TurnConfig, run_turn_with,
};
use ove_core::ops::Op;
use ove_core::revision::Source;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker::{self, relock};

struct Running {
    cancel: Arc<AtomicBool>,
}

/// Spending and size limits for the assistant (Settings → Assistant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentLimits {
    /// Spend cap per turn in US dollars; `None`: no cap.
    pub turn_cap_usd: Option<f64>,
    /// Spend cap per project per calendar month (UTC) in US dollars; `None`: no cap.
    pub project_month_cap_usd: Option<f64>,
    /// Model requests per turn.
    pub max_requests: u32,
    /// Minutes per turn.
    pub max_minutes: u32,
}

impl Default for AgentLimits {
    fn default() -> Self {
        AgentLimits {
            turn_cap_usd: Some(ove_agent::agent::DEFAULT_TURN_CAP_USD),
            project_month_cap_usd: None,
            max_requests: ove_agent::agent::DEFAULT_MAX_REQUESTS as u32,
            max_minutes: ove_agent::agent::DEFAULT_MAX_MINUTES as u32,
        }
    }
}

impl AgentLimits {
    /// Checks the values a person typed; the message says what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        for (cap, what) in
            [(self.turn_cap_usd, "The spend cap per turn"), (self.project_month_cap_usd, "The monthly spend cap")]
        {
            if let Some(c) = cap
                && !(c.is_finite() && c > 0.0 && c <= 10_000.0)
            {
                return Err(format!("{what} must be more than $0 and at most $10,000 (leave it empty for no cap)."));
            }
        }
        if !(1..=500).contains(&self.max_requests) {
            return Err("Max requests per turn must be between 1 and 500.".into());
        }
        if !(1..=240).contains(&self.max_minutes) {
            return Err("Max minutes per turn must be between 1 and 240.".into());
        }
        Ok(())
    }
}

/// Assistant state shared by the commands and the turn thread.
pub struct Assistant {
    keys: Box<dyn KeyStore>,
    model: Mutex<String>,
    limits: Mutex<AgentLimits>,
    running: Mutex<Option<Running>>,
    answers: Mutex<HashMap<String, Sender<String>>>,
    settings_path: Option<PathBuf>,
    /// Each chat's agent session: generation quotes and confirmed spend (the tool context is rebuilt per call).
    sessions: Mutex<HashMap<String, ove_tools::Session>>,
}

impl Assistant {
    /// The stored API key, if any, and the chosen model (drafting a skill uses them; add-agent-skills-and-connectors).
    pub(crate) fn key_and_model(&self) -> (Option<String>, String) {
        let model = self.model.lock().map(|m| m.clone()).unwrap_or_else(|e| e.into_inner().clone());
        (self.keys.get().map(|(k, _)| k), model)
    }

    pub fn new(settings_dir: Option<PathBuf>) -> Self {
        Self::with_keys(Box::new(OsKeyStore), settings_dir)
    }

    pub fn with_keys(keys: Box<dyn KeyStore>, settings_dir: Option<PathBuf>) -> Self {
        let settings_path = settings_dir.map(|d| d.join("assistant.json"));
        let saved = settings_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .unwrap_or_default();
        let model = saved["model"]
            .as_str()
            .filter(|m| ove_agent::MODELS.iter().any(|x| x.id == *m))
            .map(str::to_string)
            .unwrap_or_else(|| ove_agent::MODELS[0].id.to_string());
        let limits = serde_json::from_value::<AgentLimits>(saved["limits"].clone())
            .ok()
            .filter(|l| l.validate().is_ok())
            .unwrap_or_default();
        Assistant {
            keys,
            model: Mutex::new(model),
            limits: Mutex::new(limits),
            running: Mutex::new(None),
            answers: Mutex::new(HashMap::new()),
            settings_path,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Writes the model and limits to `assistant.json` (best effort: they apply for this session either way).
    fn save_settings(&self) {
        let Some(p) = &self.settings_path else { return };
        let v = serde_json::json!({"model": *relock(&self.model), "limits": *relock(&self.limits)});
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
        if let Err(e) = std::fs::write(p, v.to_string()) {
            tracing::warn!(error = %e, "could not save the assistant settings");
        }
    }
}

/// The start of the calendar month (UTC) that `now_ms` falls in, ms since the epoch.
pub fn month_start_ms(now_ms: i64) -> i64 {
    let days = now_ms.div_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm), then back to the 1st of the month.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5; // 0-based day of the month
    (days - day) * 86_400_000
}

fn lock<T>(m: &Mutex<T>) -> AppResult<std::sync::MutexGuard<'_, T>> {
    Ok(relock(m))
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
    pub limits: AgentLimits,
}

/// The result of trying the saved key with a real (free) request.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheck {
    pub ok: bool,
    /// "Key works." or the provider's error in plain words.
    pub message: String,
}

/// Tokens and estimated cost of some turns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpendView {
    /// Input tokens (cache writes included, cache reads not).
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub cache_read_tokens: f64,
    pub cost_usd: f64,
    pub turns: f64,
}

impl From<Spend> for SpendView {
    fn from(s: Spend) -> Self {
        SpendView {
            input_tokens: (s.usage.input_tokens + s.usage.cache_creation_input_tokens) as f64,
            output_tokens: s.usage.output_tokens as f64,
            cache_read_tokens: s.usage.cache_read_input_tokens as f64,
            cost_usd: s.cost_usd,
            turns: s.turns as f64,
        }
    }
}

/// Running totals for the chat header.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    /// The chat asked about, if any.
    pub chat: Option<SpendView>,
    /// Every chat of the open project, all time.
    pub project: SpendView,
    /// Every chat of the open project this calendar month (UTC).
    pub month: SpendView,
    pub month_cap_usd: Option<f64>,
}

/// Cost and tokens of one finished turn, shown under it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TurnUsageView {
    pub duration_ms: f64,
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub cache_read_tokens: f64,
    pub cost_usd: f64,
}

/// What a turn changed on the timeline, for "Show" and "Undo this turn".
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TurnReview {
    pub turn_id: String,
    /// Project revisions the turn made, oldest first.
    pub revs: Vec<f64>,
    /// Items added, changed or removed.
    pub count: u32,
    /// Added or changed items that are still on the timeline.
    pub changed_ids: Vec<String>,
    /// Names of the items the turn removed.
    pub removed: Vec<String>,
    /// "Undo this turn" already reverted it.
    pub undone: bool,
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
    /// user, assistant, tool, question, note, reasoning (the summarized thinking), changes or usage.
    pub role: String,
    pub text: String,
    /// Tool call / question id.
    pub id: Option<String>,
    pub ok: Option<bool>,
    pub options: Vec<String>,
    /// role `usage`: the finished turn's cost and tokens.
    pub usage: Option<TurnUsageView>,
    /// role `changes`: what the turn changed.
    pub review: Option<TurnReview>,
    /// role `user`: what the message mentioned (items, ranges, media, skills).
    pub mentions: Vec<MentionChip>,
}

/// A mention as the chat input sends it: an item, media or skill by id, or a time range.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionInput {
    /// `item`, `range`, `media` or `skill`.
    pub kind: String,
    pub id: Option<String>,
    /// A range's start and end, seconds.
    pub start: Option<f64>,
    pub end: Option<f64>,
}

/// A mention as the chat shows it under a message.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionChip {
    pub kind: String,
    pub label: String,
    pub ids: Vec<String>,
    /// An image was attached.
    pub image: bool,
}

impl From<&ove_agent::Mention> for MentionChip {
    fn from(m: &ove_agent::Mention) -> Self {
        MentionChip { kind: m.kind.clone(), label: m.label.clone(), ids: m.ids.clone(), image: m.image.is_some() }
    }
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
    /// Revisions committed by tool calls since the turn last asked.
    revs: Vec<i64>,
    /// The chat and turn running (recorded as the actor of each revision).
    turn: Option<(String, String)>,
}

impl DesktopHost {
    fn with_project<T>(
        &mut self,
        f: impl FnOnce(&mut ove_store::Project) -> Result<T, String>,
        changed: bool,
    ) -> Result<T, String> {
        let state = self.app.state::<AppState>();
        let mut g = crate::projects::lock(&state.projects.open).map_err(|e| e.message)?;
        let p = g.as_mut().ok_or("No project is open in the editor anymore.")?;
        let before = p.rev();
        let out = f(p)?;
        if changed {
            // The project is locked: a new revision here is this tool's.
            if p.rev() != before {
                self.revs.push(p.rev());
            }
            crate::projects::emit(&self.app, Some(p));
        }
        Ok(out)
    }
}

/// The app as seen by registry tools: the open project, and the app calls the assistant may make.
struct DesktopApp<'a> {
    host: &'a mut DesktopHost,
}

impl ove_tools::AppHost for DesktopApp<'_> {
    fn with_open(
        &mut self,
        edit: bool,
        f: &mut dyn FnMut(&mut ove_store::Project) -> Result<(), ove_tools::Fail>,
    ) -> Result<(), ove_tools::Fail> {
        let state = self.host.app.state::<AppState>();
        let mut g =
            crate::projects::lock(&state.projects.open).map_err(|e| ove_tools::Fail::new("app.busy", e.message))?;
        let p = g
            .as_mut()
            .ok_or_else(|| ove_tools::Fail::new("app.noProject", "No project is open in the editor anymore."))?;
        let before = p.rev();
        let out = f(p);
        if edit && p.rev() != before {
            // The project is locked: a new revision here is this tool's.
            self.host.revs.push(p.rev());
            crate::projects::emit(&self.host.app, Some(p));
        }
        out
    }
    fn app(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, ove_tools::Fail> {
        match method {
            "context" => Ok(crate::control::context(&self.host.app)),
            "tasks" => crate::background::tasks_method(&self.host.app.state::<AppState>().tasks, &params)
                .map_err(|r| ove_tools::Fail::new(r.code, r.message)),
            "seek" => {
                let seconds = params["seconds"].as_f64().unwrap_or(0.0);
                let _ = AgentUiEvent::Seek { seconds }.emit(&self.host.app);
                Ok(serde_json::json!({}))
            }
            other => {
                Err(ove_tools::Fail::new("app.unavailable", format!("`{other}` is not available to the assistant")))
            }
        }
    }
    /// The chat's session (quotes and the spend the person confirmed in it).
    fn session(&self) -> Option<ove_tools::Session> {
        let chat = self.host.turn.as_ref().map(|(c, _)| c.clone()).unwrap_or_default();
        let keys = self.host.app.state::<crate::generation::Generation>().keys();
        let state = self.host.app.state::<AppState>();
        let mut sessions = relock(&state.assistant.sessions);
        let s = sessions.entry(chat).or_insert_with(|| {
            ove_tools::Session::with(ove_tools::session::config_dir(), Arc::new(ove_core::SystemClock), keys)
        });
        Some(s.clone())
    }
    /// The price goes to the person as a question card in the chat; only their "Approve" approves.
    fn ask_spend(&mut self, card: &serde_json::Value) -> Result<bool, ove_tools::Fail> {
        let id = format!("spend-{}", card["quoteId"].as_str().unwrap_or_default());
        let question = ove_tools::tools::generate::card_text(card);
        let options = vec![SPEND_APPROVE.to_string(), "Decline".to_string()];
        let _ = AgentUiEvent::Question { id: id.clone(), question: question.clone(), options: options.clone() }
            .emit(&self.host.app);
        Ok(self.host.ask_user(&id, &question, &options).is_ok_and(|a| a == SPEND_APPROVE))
    }
    /// The app's own generation worker runs the job (as for jobs started from the Generate panel).
    fn start_generation(&mut self, bundle: &std::path::Path, job: &str) -> Result<(), ove_tools::Fail> {
        crate::generation::start_saved_job(&self.host.app, bundle, job)
            .map_err(|e| ove_tools::Fail::new("worker.failed", e))
    }
    fn skills(&mut self) -> Option<ove_tools::skills::SkillStore> {
        skill_store(&self.host.app)
    }
    /// The assistant's reads show live on the timeline.
    fn notify(&mut self, actor: Option<&ove_core::revision::Actor>, activity: &ove_tools::activity::Activity) {
        crate::activity::emit(&self.host.app, actor, Source::Agent, activity);
    }
}

/// The skill store of the app's library.
pub(crate) fn skill_store(app: &AppHandle) -> Option<ove_tools::skills::SkillStore> {
    let state = app.state::<AppState>();
    crate::projects::with_library(&state.projects, |l| Ok(ove_tools::skills::SkillStore::of_library(&l.root))).ok()
}

/// The question card's answer that approves a spend.
const SPEND_APPROVE: &str = "Approve";

impl ToolHost for DesktopHost {
    fn begin_turn(&mut self, chat_id: &str, turn_id: &str) {
        self.turn = Some((chat_id.to_string(), turn_id.to_string()));
    }
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
        let before = self.with_project(|p| Ok(p.rev()), false)?;
        // Recorded as the assistant's change, not the person's.
        let v = tauri::async_runtime::block_on(crate::projects::import_paths(self.app.clone(), paths, Source::Agent))
            .map_err(|e| e.message)?;
        if v.rev as i64 != before {
            self.revs.push(v.rev as i64);
        }
        Ok(format!("Imported. The project now has {} media files and {} items.", v.media.len(), v.items.len()))
    }
    fn take_revisions(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.revs)
    }
    fn check_code(&mut self, composition: &serde_json::Value, duration: f64) -> Result<String, String> {
        self.with_project(|p| Ok(tools::check_code(Some(p), composition, duration)), false)
    }
    fn seek_viewer(&mut self, seconds: f64) -> Result<String, String> {
        let _ = AgentUiEvent::Seek { seconds }.emit(&self.app);
        Ok(format!("Showing {seconds:.2}s."))
    }
    /// Every registry tool runs its handler on the open project through `AppCtx`, except importing (the app's own
    /// import also starts probing and thumbnails) and seeking (the viewer follows an event).
    fn run_tool(
        &mut self,
        tool: &'static ove_tools::ToolSpec,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ove_agent::tools::ToolOutput, String>> {
        if matches!(tool.name, "add_media" | "seek") {
            return None;
        }
        let turn = self.turn.clone();
        let mut host = DesktopApp { host: self };
        let mut ctx = match &turn {
            Some((chat, turn)) => ove_tools::AppCtx::assistant_in(&mut host, chat, turn),
            None => ove_tools::AppCtx::assistant(&mut host),
        };
        Some(
            ove_tools::call(&mut ctx, tool.name, args)
                .map(|r| ove_agent::tools::ToolOutput { text: tool.model_text(&r), images: r.images })
                .map_err(|f| f.model_text()),
        )
    }
    fn ask_user(&mut self, question_id: &str, _question: &str, _options: &[String]) -> Result<String, String> {
        let state = self.app.state::<AppState>();
        let (tx, rx) = channel();
        relock(&state.assistant.answers).insert(question_id.to_string(), tx);
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
        limits: lock(&a.limits)?.clone(),
    })
}

fn bundle(state: &AppState) -> AppResult<PathBuf> {
    let g = crate::projects::lock(&state.projects.open)?;
    g.as_ref()
        .map(|p| p.bundle().to_path_buf())
        .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open or create a project to chat about it."))
}

#[tauri::command]
#[specta::specta]
pub async fn agent_status(app: AppHandle) -> AppResult<AgentStatus> {
    worker::blocking("agent_status", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        status(&state.assistant)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_set_key(app: AppHandle, key: String) -> AppResult<AgentStatus> {
    worker::blocking("agent_set_key", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        state.assistant.keys.set(&key).map_err(|m| AppError::new(ErrorCode::InvalidArgument, m))?;
        tracing::info!("assistant API key stored in the system credential store");
        status(&state.assistant)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_clear_key(app: AppHandle) -> AppResult<AgentStatus> {
    worker::blocking("agent_clear_key", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        state.assistant.keys.clear().map_err(|m| AppError::new(ErrorCode::Io, m))?;
        status(&state.assistant)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_set_model(app: AppHandle, model: String) -> AppResult<AgentStatus> {
    worker::blocking("agent_set_model", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        if !ove_agent::MODELS.iter().any(|m| m.id == model) {
            return Err(AppError::new(ErrorCode::InvalidArgument, "Unknown model."));
        }
        *lock(&state.assistant.model)? = model;
        state.assistant.save_settings();
        status(&state.assistant)
    })
    .await
}

/// Saves the spend caps and per-turn limits (Settings → Assistant). They apply from the next turn.
#[tauri::command]
#[specta::specta]
pub async fn agent_set_limits(app: AppHandle, limits: AgentLimits) -> AppResult<AgentStatus> {
    worker::blocking("agent_set_limits", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        limits.validate().map_err(|m| AppError::new(ErrorCode::InvalidArgument, m))?;
        *lock(&state.assistant.limits)? = limits;
        state.assistant.save_settings();
        status(&state.assistant)
    })
    .await
}

/// Tries the saved key with the smallest real request there is (counting the tokens of one word: free, nothing
/// generated), so a wrong key shows up before the first turn.
#[tauri::command]
#[specta::specta]
pub async fn agent_check_key(app: AppHandle) -> AppResult<KeyCheck> {
    worker::blocking("agent_check_key", move || {
        let state = app.state::<AppState>();
        let a = &state.assistant;
        let Some((key, _)) = a.keys.get() else {
            return Ok(KeyCheck { ok: false, message: "No key is saved.".into() });
        };
        let model = lock(&a.model)?.clone();
        Ok(match Client::new(key).check_key(&model) {
            Ok(()) => KeyCheck { ok: true, message: "Key works.".into() },
            Err(e) => {
                tracing::info!(code = e.code, "the assistant key check failed");
                KeyCheck { ok: false, message: e.message }
            }
        })
    })
    .await
}

/// Running token and cost totals: the chat, the project, and the project this month.
#[tauri::command]
#[specta::specta]
pub async fn agent_usage(app: AppHandle, chat_id: Option<String>) -> AppResult<UsageTotals> {
    worker::blocking("agent_usage", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        let month_cap_usd = lock(&state.assistant.limits)?.project_month_cap_usd;
        let Ok(b) = bundle(state) else {
            return Ok(UsageTotals {
                chat: None,
                project: SpendView::default(),
                month: SpendView::default(),
                month_cap_usd,
            });
        };
        let store = ChatStore::for_bundle(&b);
        let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
        Ok(UsageTotals {
            chat: chat_id.filter(|c| !c.is_empty()).map(|c| store.spend(&c, 0).into()),
            project: store.project_spend(0).into(),
            month: store.project_spend(month_start_ms(now)).into(),
            month_cap_usd,
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_chats(app: AppHandle) -> AppResult<Vec<ChatSummaryView>> {
    worker::blocking("agent_chats", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        let Ok(b) = bundle(state) else { return Ok(vec![]) };
        Ok(ChatStore::for_bundle(&b)
            .list()
            .into_iter()
            .map(|c| ChatSummaryView { id: c.id, title: c.title, updated_at: c.updated_at as f64 })
            .collect())
    })
    .await
}

const SUMMARY_NOTE: &str = "Earlier messages are sent to the assistant as a summary to keep this chat within the model's limits. You still see all of them here.";

/// A chat's records as display items: messages, tool chips, questions, each finished turn's cost, and a "changes"
/// row after every turn that changed the project.
pub fn chat_items(records: &[Record]) -> Vec<ChatItem> {
    let item = |role: &str, text: String| ChatItem {
        role: role.into(),
        text,
        id: None,
        ok: None,
        options: vec![],
        usage: None,
        review: None,
        mentions: vec![],
    };
    let undone: HashSet<&str> = records
        .iter()
        .filter_map(|r| if let Record::TurnUndone { turn, .. } = r { Some(turn.as_str()) } else { None })
        .collect();
    let mut turn: Option<String> = None;
    let mut revs: Vec<i64> = Vec::new();
    let flush = |out: &mut Vec<ChatItem>, turn: &Option<String>, revs: &mut Vec<i64>| {
        if revs.is_empty() {
            return;
        }
        let id = turn.clone().unwrap_or_default();
        out.push(ChatItem {
            id: Some(id.clone()),
            review: Some(TurnReview {
                undone: undone.contains(id.as_str()),
                turn_id: id,
                revs: revs.drain(..).map(|r| r as f64).collect(),
                ..TurnReview::default()
            }),
            ..item("changes", String::new())
        });
    };
    let mut out: Vec<ChatItem> = Vec::new();
    for r in records {
        match r {
            Record::User { id, text, .. } => {
                flush(&mut out, &turn, &mut revs);
                turn = Some(id.clone());
                out.push(item("user", text.clone()));
            }
            Record::Revisions { turn: t, revs: made } if turn.as_deref() == Some(t.as_str()) => revs.extend(made),
            Record::Assistant { blocks, .. } => {
                for b in blocks {
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(item("assistant", t.to_string()));
                            }
                        }
                        Some("thinking") => {
                            let t = b["thinking"].as_str().unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(item("reasoning", t.to_string()));
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
            Record::Result { is_error, text, note, cost_usd, duration_ms, thread_usage: u, .. } => {
                flush(&mut out, &turn, &mut revs);
                if *is_error {
                    out.push(item("note", text.clone()));
                }
                if let Some(n) = note {
                    out.push(item("note", n.clone()));
                }
                out.push(ChatItem {
                    usage: Some(TurnUsageView {
                        duration_ms: *duration_ms as f64,
                        input_tokens: (u.input_tokens + u.cache_creation_input_tokens) as f64,
                        output_tokens: u.output_tokens as f64,
                        cache_read_tokens: u.cache_read_input_tokens as f64,
                        cost_usd: *cost_usd,
                    }),
                    ..item("usage", String::new())
                });
            }
            Record::Mentions { mentions, .. } => {
                if let Some(u) = out.iter_mut().rev().find(|i| i.role == "user") {
                    u.mentions = mentions.iter().map(MentionChip::from).collect();
                }
            }
            Record::Summary { .. } if !out.iter().any(|i| i.role == "note" && i.text == SUMMARY_NOTE) => {
                out.push(item("note", SUMMARY_NOTE.into()));
            }
            _ => {}
        }
    }
    flush(&mut out, &turn, &mut revs);
    out
}

/// A removed item's name, from the item as it was (an `AddItem` in the revision's inverse).
fn item_name(item: &serde_json::Value) -> String {
    ["name", "textContent", "captionText"]
        .iter()
        .find_map(|k| item[k].as_str().filter(|s| !s.trim().is_empty()))
        .map(|s| s.chars().take(40).collect())
        .or_else(|| item["type"].as_str().map(|t| format!("a {t} item")))
        .unwrap_or_else(|| "an item".into())
}

/// Fills in what a turn changed from its revision records: items added, changed or removed (an item added and
/// removed within the turn counts as nothing), and which of the changed ones are on the timeline now.
pub fn fill_review(r: &mut TurnReview, bundle: &Path, present: &HashSet<String>) {
    let mut touched: BTreeSet<String> = BTreeSet::new();
    let mut added: BTreeSet<String> = BTreeSet::new();
    let mut removed: Vec<(String, String)> = Vec::new();
    for rev in &r.revs {
        let Ok(rec) = ove_store::project::read_record(bundle, *rev as i64) else { continue };
        for op in &rec.ops {
            match op {
                Op::AddItem { item } => {
                    let Some(id) = item["id"].as_str() else { continue };
                    if let Some(i) = removed.iter().position(|(r, _)| r == id) {
                        removed.remove(i);
                    } else {
                        added.insert(id.to_string());
                    }
                    touched.insert(id.to_string());
                }
                Op::UpdateItem { id, .. } | Op::UnsetItemFields { id, .. } => {
                    touched.insert(id.clone());
                }
                Op::RemoveItems { ids } => {
                    for id in ids {
                        touched.remove(id);
                        if added.remove(id) {
                            continue;
                        }
                        let was = rec.inverse.iter().find_map(|o| match o {
                            Op::AddItem { item } if item["id"].as_str() == Some(id.as_str()) => Some(item_name(item)),
                            _ => None,
                        });
                        removed.push((id.clone(), was.unwrap_or_else(|| "an item".into())));
                    }
                }
                _ => {}
            }
        }
    }
    r.count = (touched.len() + removed.len()) as u32;
    r.changed_ids = touched.into_iter().filter(|id| present.contains(id)).collect();
    r.removed = removed.into_iter().map(|(_, n)| n).collect();
}

/// Reverts the revisions of one turn as a single new revision: their inverse operations, newest first, applied to
/// the project as it is now (later edits to other items are kept). Fails, changing nothing, when later edits touched
/// the same items in a way that can't be reverted.
pub fn undo_turn(p: &mut ove_store::Project, revs: &[i64]) -> AppResult<i64> {
    use ove_store::undo::UndoError;
    let mut sorted = revs.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let n = sorted.len();
    let summary = format!("undo the assistant's turn ({n} change{})", if n == 1 { "" } else { "s" });
    let later = |what: String| {
        AppError::new(
            ErrorCode::InvalidArgument,
            format!(
                "This turn can't be undone in one step because later edits changed the same items ({what}). Open History (the clock next to Undo) to go back to before it."
            ),
        )
    };
    match p.undo_selected(&sorted, Source::Undo, summary, None) {
        Ok(c) => Ok(c.record.rev),
        Err(UndoError::Nothing) => Err(AppError::new(ErrorCode::InvalidArgument, "This turn made no changes to undo.")),
        Err(UndoError::Conflict(c)) => Err(later(
            c.iter()
                .map(|x| format!("{} {}", x.item.as_deref().unwrap_or(&x.object), x.field))
                .collect::<Vec<_>>()
                .join(", "),
        )),
        Err(UndoError::Store(e)) if e.code() == "undo.unavailable" => Err(AppError::new(
            ErrorCode::NotFound,
            "This turn's changes are no longer in the project's history, so it can't be undone in one step.",
        )),
        Err(UndoError::Store(e)) => Err(later(e.to_string())),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn agent_chat(app: AppHandle, id: String) -> AppResult<Vec<ChatItem>> {
    worker::blocking("agent_chat", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        let b = bundle(state)?;
        let mut items = chat_items(&ChatStore::for_bundle(&b).records(&id));
        if items.iter().any(|i| i.review.is_some()) {
            let present: HashSet<String> = crate::projects::lock(&state.projects.open)?
                .as_ref()
                .map(|p| p.doc().project.items.keys().cloned().collect())
                .unwrap_or_default();
            for r in items.iter_mut().filter_map(|i| i.review.as_mut()) {
                fill_review(r, &b, &present);
            }
        }
        Ok(items)
    })
    .await
}

/// "Undo this turn": reverts every revision the turn made as one new revision (itself undoable) and notes it in the
/// chat log so the chat shows it after a reload.
#[tauri::command]
#[specta::specta]
pub async fn agent_undo_turn(
    app: AppHandle,
    chat_id: String,
    turn_id: String,
) -> AppResult<crate::projects::ProjectView> {
    crate::projects::serial(&app, "agent_undo_turn", move |app, state| {
        if lock(&state.assistant.running)?.is_some() {
            return Err(AppError::new(
                ErrorCode::InvalidArgument,
                "Wait for the assistant to finish (or stop it) first.",
            ));
        }
        let b = bundle(state)?;
        let store = ChatStore::for_bundle(&b);
        let records = store.records(&chat_id);
        if records.iter().any(|r| matches!(r, Record::TurnUndone { turn, .. } if *turn == turn_id)) {
            return Err(AppError::new(ErrorCode::InvalidArgument, "This turn is already undone."));
        }
        let revs: Vec<i64> = records
            .iter()
            .filter_map(|r| match r {
                Record::Revisions { turn, revs } if *turn == turn_id => Some(revs.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        if revs.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidArgument, "This turn made no changes to undo."));
        }
        let mut g = crate::projects::lock(&state.projects.open)?;
        let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        // Media the turn generated is filled in later by the generation worker, as the turn's own revision.
        let mut revs = revs;
        let first = revs.iter().min().copied().unwrap_or_default();
        for r in p.records()?.into_iter().filter(|r| r.rev > first && r.source != Source::Undo) {
            let ours = r.actor.as_ref().is_some_and(|a| {
                a.kind == ove_core::revision::ActorKind::Assistant
                    && a.session.as_deref() == Some(chat_id.as_str())
                    && a.turn.as_deref() == Some(turn_id.as_str())
            });
            if ours && !revs.contains(&r.rev) {
                revs.push(r.rev);
            }
        }
        let rev = undo_turn(p, &revs)?;
        if let Err(e) = store.append(&chat_id, &Record::TurnUndone { turn: turn_id, rev }) {
            tracing::warn!(error = %e, "could not note the undone turn in the chat log");
        }
        crate::projects::emit_view(app, Some(p))
            .ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
    })
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub chat_id: String,
    /// Things the chat tells the person about the message (images left out, mentions that no longer exist).
    #[serde(default)]
    pub notices: Vec<String>,
}

/// Resolves the message's mentions when it is sent (design D1): item and range lines and media rows from the open
/// project with its revision, then — through the tool registry, as the assistant — the attached images
/// (`grab_frame`, downscaled) and the chosen skills (`read_skill`). Returns the mentions and notices for the chat.
fn resolve_mentions(app: &AppHandle, refs: &[MentionInput]) -> AppResult<(Vec<ove_agent::Mention>, Vec<String>)> {
    if refs.is_empty() {
        return Ok((vec![], vec![]));
    }
    let to_ref =
        |m: &MentionInput| ove_agent::MentionRef { kind: m.kind.clone(), id: m.id.clone(), start: m.start, end: m.end };
    let timeline: Vec<ove_agent::MentionRef> = refs.iter().filter(|m| m.kind != "skill").map(to_ref).collect();
    let mut resolved = {
        let state = app.state::<AppState>();
        let g = crate::projects::lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open a project first."))?;
        ove_agent::mention::resolve(p, &timeline)
    };
    let mut host = DesktopHost { app: app.clone(), cancel: Arc::new(AtomicBool::new(false)), revs: vec![], turn: None };
    let mut desk = DesktopApp { host: &mut host };
    let mut ctx = ove_tools::AppCtx::assistant(&mut desk);
    for want in &resolved.images {
        let mut args = serde_json::Map::new();
        args.insert("media".into(), want.media.clone().into());
        args.insert("width".into(), want.width.into());
        let label = resolved.mentions[want.index].label.clone();
        match ove_tools::call(&mut ctx, "grab_frame", &args) {
            Ok(r) => match r.images.first() {
                Some(png) => resolved.mentions[want.index].image = Some(ove_tools::receipt::base64(&png.bytes)),
                None => resolved.notices.push(format!("{label} could not be attached as an image.")),
            },
            Err(f) => resolved.notices.push(format!("{label} could not be attached as an image: {}", f.message)),
        }
    }
    for m in refs.iter().filter(|m| m.kind == "skill") {
        let Some(id) = m.id.as_deref() else { continue };
        let mut args = serde_json::Map::new();
        args.insert("id".into(), id.into());
        match ove_tools::call(&mut ctx, "read_skill", &args) {
            Ok(r) => {
                let name = r.json["name"].as_str().unwrap_or(id).to_string();
                let text = r.json["text"].as_str().unwrap_or_default();
                resolved.mentions.push(ove_agent::mention::skill(id, &name, text));
            }
            Err(f) => resolved.notices.push(format!("The skill {id} could not be read: {}", f.message)),
        }
    }
    Ok((resolved.mentions, resolved.notices))
}

/// Starts a turn in a chat (a new chat when `chat_id` is empty). Progress arrives as `agentUiEvent`.
#[tauri::command]
#[specta::specta]
pub async fn agent_send(app: AppHandle, chat_id: Option<String>, text: String) -> AppResult<SendResult> {
    worker::blocking("agent_send", move || send(app, chat_id, text, vec![])).await
}

/// [`agent_send`] with what the message mentions (items, ranges, media, skills). Each mention is resolved now, as
/// the exact printed lines with their revision, and kept in the chat log.
#[tauri::command]
#[specta::specta]
pub async fn agent_send_context(
    app: AppHandle,
    chat_id: Option<String>,
    text: String,
    mentions: Vec<MentionInput>,
) -> AppResult<SendResult> {
    worker::blocking("agent_send_context", move || send(app, chat_id, text, mentions)).await
}

fn send(app: AppHandle, chat_id: Option<String>, text: String, refs: Vec<MentionInput>) -> AppResult<SendResult> {
    {
        let state = app.state::<AppState>();
        let state = &*state;
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidArgument, "Type a message first."));
        }
        let a = &state.assistant;
        let Some((key, _)) = a.keys.get() else {
            return Err(AppError::new(ErrorCode::InvalidArgument, "Add your Anthropic API key to use the assistant."));
        };
        let bundle = bundle(state)?;
        let model = lock(&a.model)?.clone();
        let limits = lock(&a.limits)?.clone();
        let store = ChatStore::for_bundle(&bundle);
        // The project's monthly cap is checked before anything is sent (or logged).
        let project_cap = match limits.project_month_cap_usd {
            Some(cap) => {
                let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
                let spent = store.project_spend(month_start_ms(now)).cost_usd;
                if spent >= cap {
                    return Err(AppError::new(
                        ErrorCode::InvalidArgument,
                        format!(
                            "This project reached its monthly assistant spend cap of {} (≈{} spent this month). Raise \"Spend cap per project per month\" in Settings → Assistant to continue.",
                            ove_agent::agent::dollars(cap),
                            ove_agent::agent::dollars(spent)
                        ),
                    ));
                }
                Some(ProjectCap { cap_usd: cap, spent_usd: spent })
            }
            None => None,
        };
        let (mentions, notices) = resolve_mentions(&app, &refs)?;
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
                return Err(AppError::new(
                    ErrorCode::InvalidArgument,
                    "The assistant is still working. Stop it or wait.",
                ));
            }
            *running = Some(Running { cancel: cancel.clone() });
        }
        let (app2, chat2) = (app.clone(), chat_id.clone());
        std::thread::Builder::new()
            .name("assistant-turn".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let client = Client::new(key);
                    let mut host =
                        DesktopHost { app: app2.clone(), cancel: cancel.clone(), revs: Vec::new(), turn: None };
                    let cfg = TurnConfig {
                        model,
                        max_requests: limits.max_requests as usize,
                        max_duration: Duration::from_secs(u64::from(limits.max_minutes) * 60),
                        max_cost_usd: limits.turn_cap_usd,
                        project_cap,
                        // Read each turn: a skill installed between turns shows in the next one.
                        skill_index: skill_store(&app2).and_then(|s| ove_tools::skills::index_for(&s)),
                        ..TurnConfig::default()
                    };
                    let _ =
                        run_turn_with(&client, &store, &chat2, &text, &mentions, &mut host, &cfg, &cancel, &mut |e| {
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
                *relock(&st.assistant.running) = None;
                relock(&st.assistant.answers).clear();
            })
            .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(SendResult { chat_id, notices })
    }
}

#[tauri::command]
#[specta::specta]
pub fn agent_stop(state: tauri::State<'_, AppState>) -> AppResult<()> {
    if let Some(r) = lock(&state.assistant.running)?.as_ref() {
        r.cancel.store(true, Ordering::SeqCst);
    }
    lock(&state.assistant.answers)?.clear();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn agent_answer(state: tauri::State<'_, AppState>, question_id: String, answer: String) -> AppResult<()> {
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
                    json!({"type": "thinking", "thinking": "", "signature": "s"}),
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

    fn project_with_items() -> (tempfile::TempDir, ove_store::Project) {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("T").unwrap();
        p.apply(
            vec![
                Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 0, "type": "video"}) },
                Op::AddItem {
                    item: json!({"id": "b", "trackId": "t_main", "type": "text", "startTime": 0, "endTime": 2, "name": "Kept", "textContent": "B"}),
                },
                Op::AddItem {
                    item: json!({"id": "c", "trackId": "t_main", "type": "text", "startTime": 2, "endTime": 4, "name": "Old outro", "textContent": "C"}),
                },
            ],
            Source::Gui,
            None,
        )
        .unwrap();
        (d, p)
    }

    #[test]
    fn a_turn_is_reviewed_and_undone_in_one_step_keeping_later_edits() {
        let (_d, mut p) = project_with_items();
        let mut set = serde_json::Map::new();
        set.insert("textContent".into(), json!("B2"));
        let r1 = p
            .apply(
                vec![Op::AddItem {
                    item: json!({"id": "a", "trackId": "t_main", "type": "text", "startTime": 4, "endTime": 6, "name": "Title"}),
                }],
                Source::Agent,
                Some("assistant: added 1 item".into()),
            )
            .unwrap()
            .record
            .rev;
        let r2 = p
            .apply(
                vec![Op::UpdateItem { id: "b".into(), set }, Op::RemoveItems { ids: vec!["c".into()] }],
                Source::Agent,
                Some("assistant: updated 1, removed 1".into()),
            )
            .unwrap()
            .record
            .rev;
        // The person renames the project afterwards; undoing the turn keeps that.
        let mut name = serde_json::Map::new();
        name.insert("name".into(), json!("Renamed"));
        p.apply(vec![Op::UpdateProject { set: name, unset: vec![] }], Source::Gui, None).unwrap();

        let records = vec![
            Record::User { id: "u_1".into(), text: "tidy up".into(), time: 0 },
            Record::Revisions { turn: "u_1".into(), revs: vec![r1] },
            Record::Revisions { turn: "u_1".into(), revs: vec![r2] },
            Record::Result {
                is_error: false,
                duration_ms: 1500,
                cost_usd: 0.02,
                text: "Done".into(),
                thread_usage: ove_agent::Usage { input_tokens: 100, output_tokens: 20, ..Default::default() },
                time: 1,
                note: Some("Stopped: this turn reached its spend cap of $0.01".into()),
            },
        ];
        let items = chat_items(&records);
        let roles: Vec<&str> = items.iter().map(|i| i.role.as_str()).collect();
        assert_eq!(roles, ["user", "changes", "note", "usage"]);
        assert_eq!(items[3].usage.as_ref().unwrap().cost_usd, 0.02);
        let mut review = items[1].review.clone().unwrap();
        assert_eq!(review.revs, vec![r1 as f64, r2 as f64]);
        let present: HashSet<String> = p.doc().project.items.keys().cloned().collect();
        fill_review(&mut review, p.bundle(), &present);
        assert_eq!(review.count, 3);
        assert_eq!(review.changed_ids, ["a", "b"]);
        assert_eq!(review.removed, ["Old outro"]);
        assert!(!review.undone);

        let rev = undo_turn(&mut p, &[r1, r2]).unwrap();
        let doc = p.doc();
        assert!(!doc.project.items.contains_key("a"));
        assert_eq!(doc.project.items["b"].fields["textContent"], "B");
        assert_eq!(doc.project.items["c"].name(), "Old outro");
        assert_eq!(doc.project.name, "Renamed", "later edits stay");
        let rec = p.latest_record().unwrap().unwrap();
        assert_eq!((rec.rev, rec.source), (rev, Source::Undo));
        assert!(p.session().can_undo(), "undoing the turn is itself undoable");
        // Twice cannot work: the turn's items are already back.
        let e = undo_turn(&mut p, &[r1, r2]).unwrap_err();
        assert!(e.message.contains("History"), "{}", e.message);
        // The log notes the undo, so a reload shows it.
        let mut records = records;
        records.push(Record::TurnUndone { turn: "u_1".into(), rev });
        assert!(chat_items(&records)[1].review.as_ref().unwrap().undone);
    }

    #[test]
    fn a_reopened_chat_shows_the_mentions_of_each_message() {
        let m = ove_agent::Mention {
            kind: "media".into(),
            ids: vec!["m1".into()],
            rev: Some(3),
            label: "poster.png".into(),
            excerpt: "row".into(),
            range: None,
            more: 0,
            image: Some("AAAA".into()),
        };
        let items = chat_items(&[
            Record::User { id: "u_1".into(), text: "use this".into(), time: 0 },
            Record::Mentions { turn: "u_1".into(), mentions: vec![m] },
            Record::Assistant {
                message_id: "m".into(),
                model: "x".into(),
                blocks: vec![json!({"type": "thinking", "thinking": "Compare the posters first.", "signature": "s"})],
            },
        ]);
        assert_eq!(items.len(), 2);
        assert_eq!((items[1].role.as_str(), items[1].text.as_str()), ("reasoning", "Compare the posters first."));
        let chips = &items[0].mentions;
        assert_eq!((chips[0].kind.as_str(), chips[0].label.as_str(), chips[0].image), ("media", "poster.png", true));
    }

    #[test]
    fn summaries_show_once_and_turns_without_changes_have_no_review() {
        let items = chat_items(&[
            Record::User { id: "u_1".into(), text: "hi".into(), time: 0 },
            Record::Summary { first_kept: 1, text: "s".into(), time: 0 },
            Record::Summary { first_kept: 1, text: "t".into(), time: 0 },
            Record::Revisions { turn: "other".into(), revs: vec![5] },
        ]);
        assert_eq!(items.iter().map(|i| i.role.as_str()).collect::<Vec<_>>(), ["user", "note"]);
    }

    #[test]
    fn limits_validate_persist_and_start_months_in_utc() {
        assert!(AgentLimits::default().validate().is_ok());
        assert_eq!(AgentLimits::default().turn_cap_usd, Some(2.0));
        assert_eq!((AgentLimits::default().max_requests, AgentLimits::default().max_minutes), (30, 15));
        assert!(AgentLimits { turn_cap_usd: Some(0.0), ..Default::default() }.validate().is_err());
        assert!(AgentLimits { project_month_cap_usd: Some(f64::NAN), ..Default::default() }.validate().is_err());
        assert!(AgentLimits { max_requests: 0, ..Default::default() }.validate().is_err());
        let d = tempfile::tempdir().unwrap();
        let a = Assistant::with_keys(Box::new(ove_agent::MemoryKeyStore::default()), Some(d.path().into()));
        let mine =
            AgentLimits { turn_cap_usd: None, project_month_cap_usd: Some(25.0), max_requests: 50, max_minutes: 5 };
        *relock(&a.limits) = mine.clone();
        a.save_settings();
        let again = Assistant::with_keys(Box::new(ove_agent::MemoryKeyStore::default()), Some(d.path().into()));
        assert_eq!(status(&again).unwrap().limits, mine);
        assert_eq!(month_start_ms(1_794_497_400_000), 1_793_491_200_000); // 2026-11-12 -> 2026-11-01
        assert_eq!(month_start_ms(1_709_251_140_000), 1_706_745_600_000); // 2024-02-29 -> 2024-02-01
        assert_eq!(month_start_ms(1_798_761_600_000), 1_798_761_600_000); // 2027-01-01 00:00
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
