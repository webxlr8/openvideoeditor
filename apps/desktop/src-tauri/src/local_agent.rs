//! Chat through an agent installed on this computer, over the Agent Client Protocol (add-local-agents-and-onboarding
//! design D5). The chat panel, its records, turn summaries, Show and Undo this turn are the same as for the built-in
//! assistant; only who answers differs.
//!
//! - One agent session per chat, started on the chat's first turn. Its only MCP server is `ove mcp` pinned to the
//!   open project with a session token (`--turn`): the app attributes every edit that carries the token to the turn
//!   that is running in that chat, and refuses edits with the token when no turn is running or the project is not
//!   the session's (design D6 layer 3).
//! - The harmless-agent rules of the client (no file system, no terminal, permissions only for the app's tools, an
//!   empty working folder, a time limit, Stop) are in [`ove_agents::acp`].
//! - Subscription agents report no cost: such a turn shows "uses your <agent> plan" where a price would be.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ove_agent::{ChatStore, Record};
use ove_agents::acp::{self, AcpError, Allow, Event, McpServer, Session, Stop};
use serde_json::json;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::assistant::AgentUiEvent;
use crate::worker::relock;

/// A turn's link to the edits that arrive from its MCP server (keyed by the session's token).
struct TurnLink {
    bundle: PathBuf,
    running: bool,
    revs: Vec<i64>,
}

struct Live {
    agent_id: String,
    bundle: PathBuf,
    token: String,
    session: Session,
    turns: usize,
}

/// The local-agent sessions of the app.
#[derive(Default)]
pub struct LocalAgents {
    /// By chat id.
    sessions: Mutex<HashMap<String, Live>>,
    links: Mutex<HashMap<String, TurnLink>>,
    /// Agents whose last start failed for sign-in (by id), until a session starts.
    needs_sign_in: Mutex<HashSet<String>>,
}

impl LocalAgents {
    /// Checks an edit that carries `token` before it is applied: the token's turn must be running, on `bundle`.
    pub fn admit(&self, token: &str, bundle: &Path) -> Result<(), ove_control::Refusal> {
        let links = relock(&self.links);
        match links.get(token) {
            Some(l) if l.running && same_bundle(&l.bundle, bundle) => Ok(()),
            Some(l) if l.running => {
                Err(ove_control::Refusal::new("turn.otherProject", "this assistant session works on another project"))
            }
            Some(_) => Err(ove_control::Refusal::new(
                "turn.notRunning",
                "the assistant's turn has ended; edits are made during a turn only",
            )),
            None => Err(ove_control::Refusal::new("turn.unknown", "not an assistant session of this app")),
        }
    }

    /// Notes a revision made for the turn of `token`.
    pub fn record(&self, token: &str, rev: i64) {
        if let Some(l) = relock(&self.links).get_mut(token) {
            l.revs.push(rev);
        }
    }

    pub fn needs_sign_in(&self, agent_id: &str) -> bool {
        relock(&self.needs_sign_in).contains(agent_id)
    }

    /// Ends every agent process (app exit).
    pub fn close_all(&self) {
        let mut s = relock(&self.sessions);
        for (_, mut live) in s.drain() {
            live.session.close();
        }
        relock(&self.links).clear();
    }
}

fn same_bundle(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

fn token() -> String {
    ove_control::new_token()
}

/// Test hook: start a stand-in program instead of an agent (`OVE_AGENT_OVERRIDE` =
/// `{"id": "claude", "program": "…", "args": […]}`), only in debug builds or with `OVE_DEV_ENDPOINTS=1`, like the
/// other development endpoints. Used by the app end-to-end suite.
pub fn override_launch(id: &str) -> Option<acp::Launch> {
    if !(cfg!(debug_assertions) || std::env::var("OVE_DEV_ENDPOINTS").is_ok_and(|v| v == "1")) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&std::env::var("OVE_AGENT_OVERRIDE").ok()?).ok()?;
    (v["id"].as_str()? == id).then(|| acp::Launch {
        program: PathBuf::from(v["program"].as_str().unwrap_or_default()),
        args: v["args"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        env: vec![],
    })
}

fn launch_for(spec: &ove_agents::AgentSpec) -> Result<acp::Launch, String> {
    if let Some(l) = override_launch(spec.id) {
        return Ok(l);
    }
    let env = ove_agents::Env::from_process();
    let found = ove_agents::detect(spec, &env, &ove_agents::Limits::default());
    let program = found.path.map(PathBuf::from);
    ove_agents::launch(spec, program.as_deref(), &env).ok_or_else(|| match program {
        None => format!("{} is not installed on this computer.", spec.name),
        Some(_) => format!("{} needs Node.js (npx) on this computer to be used from the app.", spec.name),
    })
}

/// What a turn needs to know.
pub struct TurnInput {
    pub chat_id: String,
    pub agent_id: String,
    pub text: String,
    pub bundle: PathBuf,
    pub max_tool_calls: usize,
    pub max_duration: Duration,
    pub cancel: Arc<AtomicBool>,
}

fn emit(app: &AppHandle, e: AgentUiEvent) {
    let _ = e.emit(app);
}

/// The earlier conversation of a chat, for an agent session that starts after it (the agent did not see it).
fn earlier(records: &[Record]) -> Option<String> {
    let mut lines = Vec::new();
    for r in records {
        match r {
            Record::User { text, .. } => lines.push(format!("Person: {text}")),
            Record::Assistant { blocks, .. } => {
                for b in blocks {
                    if let Some(t) = b["text"].as_str().filter(|t| !t.trim().is_empty()) {
                        lines.push(format!("Assistant: {t}"));
                    }
                }
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        return None;
    }
    let mut text = String::new();
    for l in lines.iter().rev() {
        if text.len() + l.len() > 8000 {
            break;
        }
        text = format!("{l}\n{text}");
    }
    Some(text)
}

/// Runs one turn in the chat's agent session (starting the session first when needed). Appends the turn to the
/// chat log and reports it through `agentUiEvent`, like the built-in assistant.
pub fn run_turn(app: &AppHandle, t: TurnInput) {
    let started = Instant::now();
    let agents = app.state::<LocalAgents>();
    let store = ChatStore::for_bundle(&t.bundle);
    let Some(spec) = ove_agents::agent(&t.agent_id) else {
        emit(
            app,
            AgentUiEvent::Error { code: "agent.unknown".into(), message: "That assistant is not supported.".into() },
        );
        return;
    };
    let turn_id = ove_agent::agent::new_turn_id();
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
    let history = store.records(&t.chat_id);
    let _ = store.append(&t.chat_id, &Record::User { id: turn_id.clone(), text: t.text.clone(), time: now });
    store.touch(&t.chat_id, spec.name);
    emit(app, AgentUiEvent::TurnStarted { chat_id: t.chat_id.clone() });

    // The chat's session: reused while it is the same agent and project, else (re)started. It is taken out of the map
    // for the turn, so the map is never locked while the agent works.
    let previous = relock(&agents.sessions).remove(&t.chat_id);
    let mut live = match previous {
        Some(l) if l.agent_id == t.agent_id && same_bundle(&l.bundle, &t.bundle) => l,
        other => {
            if let Some(mut old) = other {
                old.session.close();
                relock(&agents.links).remove(&old.token);
            }
            match start_session(spec, &t.bundle) {
                Ok(live) => {
                    relock(&agents.needs_sign_in).remove(spec.id);
                    live
                }
                Err(e) => {
                    if matches!(e, AcpError::SignIn { .. }) {
                        relock(&agents.needs_sign_in).insert(spec.id.to_string());
                    }
                    finish(app, &store, &t, spec, &turn_id, started, "", Some(failure(spec, &e)), false);
                    return;
                }
            }
        }
    };
    let token = live.token.clone();
    relock(&agents.links).insert(token.clone(), TurnLink { bundle: t.bundle.clone(), running: true, revs: Vec::new() });

    let mut prompt = t.text.clone();
    if live.turns == 0
        && let Some(e) = earlier(&history)
    {
        prompt = format!("Earlier in this conversation:\n{e}\nNow: {}", t.text);
    }
    live.turns += 1;

    let mut text = String::new();
    let mut tools = 0usize;
    let mut over_limit = false;
    let cancel = t.cancel.clone();
    let result = live.session.prompt(&prompt, t.max_duration, &cancel, &mut |e| match e {
        Event::Text(s) => {
            text.push_str(&s);
            emit(app, AgentUiEvent::TextDelta { text: s });
        }
        Event::Thought(s) => emit(app, AgentUiEvent::ThinkingDelta { text: s }),
        Event::ToolStarted { id, title, .. } => {
            tools += 1;
            if tools > t.max_tool_calls && !over_limit {
                // Layer 6: the turn's step cap, as for the built-in assistant.
                over_limit = true;
                cancel.store(true, Ordering::SeqCst);
            }
            let label = tool_label(&title);
            let _ = store.append(&t.chat_id, &Record::AgentTool { id: id.clone(), title: label.clone(), ok: None });
            emit(app, AgentUiEvent::ToolCall { id, name: title, label });
        }
        Event::ToolStatus { id, status } if status == "completed" || status == "failed" => {
            let ok = status == "completed";
            let _ = store.append(&t.chat_id, &Record::AgentTool { id: id.clone(), title: String::new(), ok: Some(ok) });
            emit(app, AgentUiEvent::ToolResult { id, ok, summary: if ok { String::new() } else { "failed".into() } });
        }
        Event::ToolStatus { .. } | Event::Plan(_) => {}
        Event::Refused { what } => {
            let id = format!("refused-{}", ove_agent::agent::new_turn_id());
            let label = format!("Not allowed: {what}");
            let _ =
                store.append(&t.chat_id, &Record::AgentTool { id: id.clone(), title: label.clone(), ok: Some(false) });
            emit(app, AgentUiEvent::ToolCall { id: id.clone(), name: "refused".into(), label });
            emit(app, AgentUiEvent::ToolResult { id, ok: false, summary: "refused".into() });
        }
    });

    // The turn is over: no more edits for it.
    let revs = relock(&agents.links).get_mut(&token).map(|l| {
        l.running = false;
        std::mem::take(&mut l.revs)
    });
    let failed = match &result {
        Err(e) => {
            // The session is unusable after a failure; the next turn starts a new one.
            live.session.close();
            relock(&agents.links).remove(&token);
            if matches!(e, AcpError::SignIn { .. }) {
                relock(&agents.needs_sign_in).insert(spec.id.to_string());
            }
            Some(failure(spec, e))
        }
        Ok(_) => {
            relock(&agents.sessions).insert(t.chat_id.clone(), live);
            None
        }
    };
    if let Some(revs) = revs.filter(|r| !r.is_empty()) {
        let _ = store.append(&t.chat_id, &Record::Revisions { turn: turn_id.clone(), revs });
    }
    let stopped = matches!(result, Ok(Stop::Cancelled));
    let note = if over_limit {
        Some(format!("Stopped after {} tool calls (Max requests per turn in Settings → Assistant).", t.max_tool_calls))
    } else {
        failed
    };
    finish(app, &store, &t, spec, &turn_id, started, &text, note, stopped);
}

fn tool_label(title: &str) -> String {
    // `mcp__openvideoeditor__apply_timeline` → the built-in assistant's wording for the same tool.
    let bare = title.rsplit("__").next().unwrap_or(title);
    if ove_agents::APP_TOOLS.contains(&bare) { ove_agent::tools::label(bare, &json!({})) } else { title.to_string() }
}

fn failure(spec: &ove_agents::AgentSpec, e: &AcpError) -> String {
    match e {
        AcpError::SignIn { message } => format!(
            "{} needs you to sign in. Open a terminal, run `{}` and sign in, then send your message again. ({message})",
            spec.name,
            spec.programs.first().copied().unwrap_or(spec.id)
        ),
        AcpError::Start(m) => format!("{} could not be started: {m}", spec.name),
        other => format!("{}: {other}", spec.name),
    }
}

#[allow(clippy::too_many_arguments)]
fn finish(
    app: &AppHandle,
    store: &ChatStore,
    t: &TurnInput,
    spec: &ove_agents::AgentSpec,
    turn_id: &str,
    started: Instant,
    text: &str,
    note: Option<String>,
    stopped: bool,
) {
    let duration_ms = started.elapsed().as_millis() as u64;
    if !text.trim().is_empty() {
        let _ = store.append(
            &t.chat_id,
            &Record::Assistant {
                message_id: format!("{turn_id}_a"),
                model: spec.name.into(),
                blocks: vec![json!({"type": "text", "text": text})],
            },
        );
    }
    let is_error = note.is_some() && text.trim().is_empty() && !stopped;
    let _ = store.append(
        &t.chat_id,
        &Record::Result {
            is_error,
            duration_ms,
            cost_usd: 0.0,
            text: if is_error { note.clone().unwrap_or_default() } else { String::new() },
            thread_usage: ove_agent::Usage::default(),
            time: ove_core::Clock::now_ms(&ove_core::SystemClock),
            note: if is_error { None } else { note.clone() },
            plan: Some(spec.name.into()),
        },
    );
    store.touch(&t.chat_id, spec.name);
    if is_error && let Some(n) = &note {
        emit(app, AgentUiEvent::Error { code: "agent.failed".into(), message: n.clone() });
    }
    emit(
        app,
        AgentUiEvent::TurnDone {
            chat_id: t.chat_id.clone(),
            stopped,
            duration_ms: duration_ms as f64,
            input_tokens: 0.0,
            output_tokens: 0.0,
            cache_read_tokens: 0.0,
            cost_usd: 0.0,
            note: if is_error { None } else { note },
            plan: Some(spec.name.into()),
        },
    );
}

fn start_session(spec: &ove_agents::AgentSpec, bundle: &Path) -> Result<Live, AcpError> {
    let launch = launch_for(spec).map_err(AcpError::Start)?;
    let library = ove_store::library::resolve_library(None).map_err(|e| AcpError::Start(e.to_string()))?;
    let token = token();
    let server = McpServer {
        name: ove_agents::SERVER_NAME.into(),
        command: crate::agents::ove_program(&ove_agents::Env::from_process()),
        args: vec![
            "--library".into(),
            library.display().to_string(),
            "mcp".into(),
            "--project".into(),
            bundle.display().to_string(),
            "--turn".into(),
            token.clone(),
        ],
    };
    let allow = Allow {
        server: ove_agents::SERVER_NAME.into(),
        tools: ove_agents::APP_TOOLS.iter().map(|s| s.to_string()).collect(),
    };
    // Starting may download an adapter the first time (npx), so the start gets more time than a step.
    let session = Session::start(&launch, &server, allow, Duration::from_secs(180))?;
    tracing::info!(agent = spec.id, session = %session.session_id, name = %session.agent_name, "local agent session started");
    Ok(Live { agent_id: spec.id.into(), bundle: bundle.to_path_buf(), token, session, turns: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_are_admitted_only_for_a_running_turn_on_its_project() {
        let a = LocalAgents::default();
        let d = tempfile::tempdir().unwrap();
        let (p, q) = (d.path().join("A.ove"), d.path().join("B.ove"));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::create_dir_all(&q).unwrap();
        assert_eq!(a.admit("t", &p).unwrap_err().code, "turn.unknown");
        relock(&a.links).insert("t".into(), TurnLink { bundle: p.clone(), running: true, revs: vec![] });
        assert!(a.admit("t", &p).is_ok());
        assert_eq!(a.admit("t", &q).unwrap_err().code, "turn.otherProject");
        a.record("t", 7);
        a.record("nope", 8);
        assert_eq!(relock(&a.links)["t"].revs, vec![7]);
        relock(&a.links).get_mut("t").unwrap().running = false;
        assert_eq!(a.admit("t", &p).unwrap_err().code, "turn.notRunning");
    }

    #[test]
    fn tool_titles_read_like_the_built_in_assistants() {
        assert_eq!(tool_label("mcp__openvideoeditor__print_timeline"), "Reading the timeline");
        assert_eq!(tool_label("Something else"), "Something else");
    }

    #[test]
    fn earlier_conversation_is_kept_short_and_recent() {
        let mut recs = vec![];
        for i in 0..500 {
            recs.push(Record::User { id: format!("u{i}"), text: format!("message {i} {}", "x".repeat(40)), time: i });
        }
        let e = earlier(&recs).unwrap();
        assert!(e.len() <= 8100, "{}", e.len());
        assert!(e.contains("message 499"), "the most recent is kept");
        assert!(!e.contains("message 0 "), "the oldest is dropped");
        assert!(earlier(&[]).is_none());
    }
}
