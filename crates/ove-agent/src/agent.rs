//! One assistant turn: send the conversation, stream the reply, run the tools it calls, repeat until it answers —
//! within a per-turn budget, stoppable at any point, with every step appended to the chat log.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::api::{ApiError, Client, StreamEvent, Usage, model_info, pricing_model};
use crate::chat::{
    ChatStore, QuestionItem, QuestionOption, Record, estimate_tokens, messages, summary_cut, transcript, user_indices,
};
use crate::prompt::SYSTEM_PROMPT;
use crate::tools::{ToolHost, definitions, dispatch, label};

/// Default spend cap per turn, in US dollars.
pub const DEFAULT_TURN_CAP_USD: f64 = 2.0;
/// Default model calls per turn.
pub const DEFAULT_MAX_REQUESTS: usize = 30;
/// Default minutes per turn.
pub const DEFAULT_MAX_MINUTES: u64 = 15;
/// The model that writes summaries of long chats (cheap and fast).
pub const SUMMARY_MODEL: &str = "claude-haiku-5-5";

/// A project's monthly spend cap and what it has spent this month before the turn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectCap {
    pub cap_usd: f64,
    pub spent_usd: f64,
}

/// Limits for one turn.
#[derive(Debug, Clone)]
pub struct TurnConfig {
    pub model: String,
    /// `output_config.effort`.
    pub effort: String,
    /// Model calls per turn (each tool round trip is one).
    pub max_requests: usize,
    pub max_duration: Duration,
    /// Output tokens across the turn.
    pub max_output_tokens: u64,
    /// Estimated spend per turn (checked between requests).
    pub max_cost_usd: Option<f64>,
    /// The project's monthly cap (checked before the turn and between requests).
    pub project_cap: Option<ProjectCap>,
    /// Estimated input tokens above which older turns are summarised; `None`: 60 % of the model's context window.
    pub compact_at_tokens: Option<u64>,
    /// The model that writes those summaries.
    pub summary_model: String,
}

impl Default for TurnConfig {
    fn default() -> Self {
        TurnConfig {
            model: crate::api::MODELS[0].id.into(),
            effort: "medium".into(),
            max_requests: DEFAULT_MAX_REQUESTS,
            max_duration: Duration::from_secs(DEFAULT_MAX_MINUTES * 60),
            max_output_tokens: 250_000,
            max_cost_usd: Some(DEFAULT_TURN_CAP_USD),
            project_cap: None,
            compact_at_tokens: None,
            summary_model: SUMMARY_MODEL.into(),
        }
    }
}

impl TurnConfig {
    /// Estimated input tokens above which older turns are summarised.
    pub fn compact_threshold(&self) -> u64 {
        self.compact_at_tokens.unwrap_or(model_info(&self.model).context_window * 3 / 5)
    }
}

/// Dollars as the UI shows them.
pub fn dollars(v: f64) -> String {
    format!("${v:.2}")
}

/// The plain-language reason a turn must stop before its next request, if a limit is reached.
pub fn limit_reached(cfg: &TurnConfig, requests: usize, elapsed: Duration, usage: &Usage, cost: f64) -> Option<String> {
    if let Some(p) = cfg.project_cap
        && p.spent_usd + cost >= p.cap_usd
    {
        return Some(format!(
            "Stopped: this project reached its monthly assistant spend cap of {} (≈{} spent this month). Raise \"Spend cap per project per month\" in Settings → Assistant to continue.",
            dollars(p.cap_usd),
            dollars(p.spent_usd + cost)
        ));
    }
    if let Some(cap) = cfg.max_cost_usd
        && cost >= cap
    {
        return Some(format!(
            "Stopped: this turn reached its spend cap of {} (≈{} spent). Raise \"Spend cap per turn\" in Settings → Assistant, or ask me to continue.",
            dollars(cap),
            dollars(cost)
        ));
    }
    if requests >= cfg.max_requests {
        return Some(format!(
            "Stopped: this turn used its limit of {} requests. Raise \"Max requests per turn\" in Settings → Assistant, or ask me to continue.",
            cfg.max_requests
        ));
    }
    if elapsed > cfg.max_duration {
        let mins = cfg.max_duration.as_secs().div_ceil(60);
        return Some(format!(
            "Stopped: this turn ran for its limit of {mins} minute{}. Raise \"Max minutes per turn\" in Settings → Assistant, or ask me to continue.",
            if mins == 1 { "" } else { "s" }
        ));
    }
    if usage.output_tokens > cfg.max_output_tokens {
        return Some("Stopped: this turn reached its output token budget. Ask me to continue if needed.".into());
    }
    None
}

/// Instructions for the summary call.
const SUMMARY_SYSTEM: &str = "You compress a video-editing assistant's conversation with a user so it can continue within the model's limits. Write a concise summary (at most 400 words) for the assistant itself: what the user wants overall, decisions and preferences they stated, what was changed on the timeline and what is still to do, and open questions. Leave out tool output details that can be read again (the timeline can be printed again). Do not invent anything.";

/// Summarises the oldest turns of a long chat into a [`Record::Summary`] when the next request would be larger than
/// the threshold. The newest turns (always the current one) stay verbatim, and a cut is only ever made at the start
/// of a turn, so tool calls and their results are never separated. Returns whether a summary was written. A failed
/// summary call is logged and the chat is sent in full.
pub fn compact_if_needed(
    client: &Client,
    store: &ChatStore,
    chat_id: &str,
    cfg: &TurnConfig,
    cancel: &AtomicBool,
    usage: &mut Usage,
    cost: &mut f64,
) -> Result<bool, ApiError> {
    let records = store.records(chat_id);
    let threshold = cfg.compact_threshold();
    if estimate_tokens(&request_body(cfg, &messages(&records))) <= threshold {
        return Ok(false);
    }
    let users = user_indices(&records);
    if users.is_empty() {
        return Ok(false);
    }
    let (prev_summary, from) = summary_cut(&records);
    let prev_kept = if prev_summary.is_some() {
        records
            .iter()
            .rev()
            .find_map(|r| if let Record::Summary { first_kept, .. } = r { Some(*first_kept) } else { None })
    } else {
        None
    };
    // Keep as many recent turns as fit in half the threshold (with room for the summary), at least the current one.
    let overhead = estimate_tokens(&request_body(cfg, &[])) + 1_000;
    let sizes: Vec<u64> =
        records.iter().map(|r| estimate_tokens(&serde_json::to_value(r).unwrap_or_default())).collect();
    let suffix = |from: usize| sizes[from..].iter().sum::<u64>();
    let last = users.len() - 1;
    let first_candidate = prev_kept.map_or(1, |k| k + 1);
    if first_candidate > last {
        return Ok(false);
    }
    let keep = (first_candidate..=last).find(|&k| overhead + suffix(users[k]) <= threshold / 2).unwrap_or(last);
    let mut covered: Vec<Record> = Vec::new();
    if let Some(prev) = prev_summary {
        covered.push(Record::Summary { first_kept: 0, text: prev.to_string(), time: 0 });
    }
    covered.extend(records[from..users[keep]].iter().cloned());
    let body = json!({
        "model": cfg.summary_model,
        "max_tokens": 2000,
        "stream": true,
        "system": SUMMARY_SYSTEM,
        "messages": [{"role": "user", "content": format!(
            "Here is the conversation so far:\n\n{}\n\nWrite the summary now.",
            transcript(&covered, 400_000)
        )}],
        "thinking": {"type": "disabled"},
        "output_config": {"effort": "low"},
    });
    let resp = client.stream(
        &body,
        cancel,
        &mut |e| {
            if let StreamEvent::Usage { model, usage: u } = e {
                usage.add(&u);
                *cost += u.cost_usd(pricing_model(&model, &cfg.summary_model));
            }
        },
        &mut |_, _| {},
    );
    let resp = match resp {
        Ok(r) => r,
        Err(e) if e.code == "turn.cancelled" => return Err(e),
        Err(e) => {
            tracing::warn!(code = e.code, "summarising a long chat failed; sending it in full");
            return Ok(false);
        }
    };
    let text: String = resp
        .content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    if text.trim().is_empty() {
        return Ok(false);
    }
    let summary = Record::Summary {
        first_kept: keep,
        text: text.trim().to_string(),
        time: ove_core::Clock::now_ms(&ove_core::SystemClock),
    };
    store
        .append(chat_id, &summary)
        .map_err(|e| ApiError::new("chat.io", format!("Couldn't save the conversation: {e}")))?;
    Ok(true)
}

/// A new turn id: unique within a chat log (time plus a counter).
fn new_turn_id() -> String {
    use std::sync::atomic::AtomicU32;
    static SEQ: AtomicU32 = AtomicU32::new(0);
    format!("u_{:x}_{:x}", ove_core::Clock::now_ms(&ove_core::SystemClock), SEQ.fetch_add(1, Ordering::Relaxed))
}

/// What the UI is told while a turn runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    TurnStarted { chat_id: String },
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta { text: String },
    #[serde(rename_all = "camelCase")]
    ToolCall { id: String, name: String, label: String },
    #[serde(rename_all = "camelCase")]
    ToolResult { id: String, ok: bool, summary: String },
    #[serde(rename_all = "camelCase")]
    Question { id: String, question: String, options: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Retrying { message: String, seconds: f64 },
    #[serde(rename_all = "camelCase")]
    TurnDone { chat_id: String, stopped: bool, duration_ms: u64, usage: Usage, cost_usd: f64, note: Option<String> },
    #[serde(rename_all = "camelCase")]
    Error { code: String, message: String },
}

/// Result of a finished turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSummary {
    /// The id of the turn's user record.
    pub turn_id: String,
    pub text: String,
    pub usage: Usage,
    pub cost_usd: f64,
    pub stopped: bool,
    pub note: Option<String>,
    /// Project revisions the turn made, oldest first.
    pub revs: Vec<i64>,
}

/// The request body for the current conversation. Tools and system prompt come first and never change, so the
/// cached prefix stays valid; the history is append-only.
pub fn request_body(cfg: &TurnConfig, msgs: &[Value]) -> Value {
    let mut body = json!({
        "model": cfg.model,
        "max_tokens": 64000,
        "stream": true,
        "system": [{"type": "text", "text": SYSTEM_PROMPT, "cache_control": {"type": "ephemeral"}}],
        "tools": definitions(),
        "messages": msgs,
        "thinking": {"type": "adaptive", "display": "summarized"},
        "output_config": {"effort": cfg.effort},
        "cache_control": {"type": "ephemeral"},
    });
    if model_info(&cfg.model).fallbacks {
        body["fallbacks"] = json!("default");
    }
    body
}

fn summarize_result(name: &str, ok: bool, out: &str) -> String {
    if !ok {
        return out.lines().next().unwrap_or("failed").chars().take(160).collect();
    }
    match name {
        "apply_timeline" => serde_json::from_str::<Value>(out)
            .ok()
            .and_then(|v| v["summary"].as_str().map(str::to_string))
            .unwrap_or_else(|| "applied".into()),
        "print_timeline" => format!("{} lines", out.lines().count()),
        "verify_timeline" => out.lines().next().unwrap_or_default().chars().take(160).collect(),
        "check_code" => serde_json::from_str::<Value>(out)
            .ok()
            .map(|v| match v["diagnostics"].as_array().map(Vec::len).unwrap_or(0) {
                0 => "no problems".to_string(),
                n => format!("{n} problem{}", if n == 1 { "" } else { "s" }),
            })
            .unwrap_or_default(),
        "list_media" => serde_json::from_str::<Value>(out)
            .ok()
            .and_then(|v| v.as_array().map(|a| format!("{} files", a.len())))
            .unwrap_or_default(),
        _ => out.chars().take(160).collect(),
    }
}

/// Runs one turn of a chat: appends the user message and everything that follows to the log.
#[allow(clippy::too_many_arguments)]
pub fn run_turn(
    client: &Client,
    store: &ChatStore,
    chat_id: &str,
    user_text: &str,
    host: &mut dyn ToolHost,
    cfg: &TurnConfig,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<TurnSummary, ApiError> {
    let started = Instant::now();
    let io = |e: std::io::Error| ApiError::new("chat.io", format!("Couldn't save the conversation: {e}"));
    let turn_id = new_turn_id();
    store
        .append(
            chat_id,
            &Record::User {
                id: turn_id.clone(),
                text: user_text.into(),
                time: ove_core::Clock::now_ms(&ove_core::SystemClock),
            },
        )
        .map_err(io)?;
    store.touch(chat_id, &cfg.model);
    emit(AgentEvent::TurnStarted { chat_id: chat_id.into() });

    let mut usage = Usage::default();
    let mut cost = 0.0;
    let mut final_text = String::new();
    let mut note: Option<String> = None;
    let mut stopped = false;
    let mut error: Option<ApiError> = None;
    let mut revs: Vec<i64> = Vec::new();

    for request in 0.. {
        if let Some(why) = limit_reached(cfg, request, started.elapsed(), &usage, cost) {
            note = Some(why);
            break;
        }
        match compact_if_needed(client, store, chat_id, cfg, cancel, &mut usage, &mut cost) {
            Ok(_) => {}
            Err(e) if e.code == "turn.cancelled" => {
                stopped = true;
                break;
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
        let body = request_body(cfg, &messages(&store.records(chat_id)));
        let shared = std::cell::RefCell::new(&mut *emit);
        let resp = client.stream(
            &body,
            cancel,
            &mut |e| match e {
                StreamEvent::TextDelta(t) => (shared.borrow_mut())(AgentEvent::TextDelta { text: t }),
                StreamEvent::ThinkingDelta(t) => (shared.borrow_mut())(AgentEvent::ThinkingDelta { text: t }),
                StreamEvent::ToolUseStart(..) => {}
                // Every attempt that started is billed, including ones that failed part-way or were stopped, at the
                // price of the model that actually served it.
                StreamEvent::Usage { model, usage: u } => {
                    usage.add(&u);
                    cost += u.cost_usd(pricing_model(&model, &cfg.model));
                }
            },
            &mut |e, wait| {
                (shared.borrow_mut())(AgentEvent::Retrying { message: e.message.clone(), seconds: wait.as_secs_f64() })
            },
        );
        let resp = match resp {
            Ok(r) => r,
            Err(e) if e.code == "turn.cancelled" => {
                stopped = true;
                break;
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        };
        store
            .append(
                chat_id,
                &Record::Assistant {
                    message_id: resp.id.clone(),
                    model: resp.model.clone(),
                    blocks: resp.content.clone(),
                },
            )
            .map_err(io)?;
        let text: String = resp
            .content
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            final_text = text;
        }
        let stop = resp.stop_reason.as_deref().unwrap_or("end_turn");
        if stop == "refusal" {
            // Never run tools from a refused turn; the history keeps the refusal as sent.
            note = Some("The model declined this request.".into());
            for b in resp.content.iter().filter(|b| b["type"] == "tool_use") {
                let id = b["id"].as_str().unwrap_or_default().to_string();
                store
                    .append(
                        chat_id,
                        &Record::ToolResult {
                            tool_use_id: id,
                            text: "Not run: the turn was declined.".into(),
                            is_error: true,
                        },
                    )
                    .map_err(io)?;
            }
            break;
        }
        let calls: Vec<&Value> = resp.content.iter().filter(|b| b["type"] == "tool_use").collect();
        if calls.is_empty() {
            break;
        }
        let truncated = stop == "max_tokens";
        for call in calls {
            let id = call["id"].as_str().unwrap_or_default().to_string();
            let name = call["name"].as_str().unwrap_or_default().to_string();
            if cancel.load(Ordering::SeqCst) {
                stopped = true;
                store
                    .append(
                        chat_id,
                        &Record::ToolResult {
                            tool_use_id: id,
                            text: "Stopped by the user before this ran.".into(),
                            is_error: true,
                        },
                    )
                    .map_err(io)?;
                continue;
            }
            let (ok, out) = if let Some((_, raw)) = resp.invalid_tool_inputs.iter().find(|(i, _)| *i == id) {
                (false, json!({"INVALID_JSON": raw}).to_string())
            } else if truncated {
                (false, "Your tool input was cut off by the output limit. Send a smaller edit.".to_string())
            } else {
                emit(AgentEvent::ToolCall { id: id.clone(), name: name.clone(), label: label(&name, &call["input"]) });
                if name == "ask_user" {
                    let q = call["input"]["question"].as_str().unwrap_or_default().to_string();
                    let opts: Vec<String> = call["input"]["options"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|o| o.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    let item = QuestionItem {
                        header: "Question".into(),
                        question: q.clone(),
                        multi_select: false,
                        options: opts
                            .iter()
                            .map(|o| QuestionOption { label: o.clone(), description: String::new() })
                            .collect(),
                    };
                    store.append(chat_id, &Record::Question { id: id.clone(), questions: vec![item] }).map_err(io)?;
                    emit(AgentEvent::Question { id: id.clone(), question: q, options: opts });
                }
                match dispatch(host, &id, &name, &call["input"]) {
                    Ok(s) => (true, s),
                    Err(e) => (false, e),
                }
            };
            emit(AgentEvent::ToolResult { id: id.clone(), ok, summary: summarize_result(&name, ok, &out) });
            store.append(chat_id, &Record::ToolResult { tool_use_id: id, text: out, is_error: !ok }).map_err(io)?;
            let made = host.take_revisions();
            if !made.is_empty() {
                store.append(chat_id, &Record::Revisions { turn: turn_id.clone(), revs: made.clone() }).map_err(io)?;
                revs.extend(made);
            }
        }
        if stopped {
            break;
        }
    }

    let duration_ms = started.elapsed().as_millis() as u64;
    let (is_error, text) = match &error {
        Some(e) => (true, e.message.clone()),
        None if stopped => (true, "Stopped.".to_string()),
        None => (false, final_text.clone()),
    };
    store
        .append(
            chat_id,
            &Record::Result {
                is_error,
                duration_ms,
                cost_usd: cost,
                text,
                thread_usage: usage,
                time: ove_core::Clock::now_ms(&ove_core::SystemClock),
                note: note.clone(),
            },
        )
        .map_err(io)?;
    store.touch(chat_id, &cfg.model);
    if let Some(e) = &error {
        emit(AgentEvent::Error { code: e.code.into(), message: e.message.clone() });
    }
    emit(AgentEvent::TurnDone {
        chat_id: chat_id.into(),
        stopped,
        duration_ms,
        usage,
        cost_usd: cost,
        note: note.clone(),
    });
    match error {
        Some(e) => Err(e),
        None => Ok(TurnSummary { turn_id, text: final_text, usage, cost_usd: cost, stopped, note, revs }),
    }
}
