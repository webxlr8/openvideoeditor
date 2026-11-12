//! One assistant turn: send the conversation, stream the reply, run the tools it calls, repeat until it answers —
//! within a per-turn budget, stoppable at any point, with every step appended to the chat log.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::api::{ApiError, Client, StreamEvent, Usage, model_info, pricing_model};
use crate::chat::{ChatStore, QuestionItem, QuestionOption, Record, messages};
use crate::prompt::SYSTEM_PROMPT;
use crate::tools::{ToolHost, definitions, dispatch, label};

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
}

impl Default for TurnConfig {
    fn default() -> Self {
        TurnConfig {
            model: crate::api::MODELS[0].id.into(),
            effort: "medium".into(),
            max_requests: 30,
            max_duration: Duration::from_secs(15 * 60),
            max_output_tokens: 250_000,
        }
    }
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
    pub text: String,
    pub usage: Usage,
    pub stopped: bool,
    pub note: Option<String>,
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
    store
        .append(
            chat_id,
            &Record::User {
                id: format!("u_{}", started.elapsed().as_nanos()),
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

    for request in 0.. {
        if request >= cfg.max_requests
            || started.elapsed() > cfg.max_duration
            || usage.output_tokens > cfg.max_output_tokens
        {
            note =
                Some("Stopped: this turn reached its step, time or token budget. Ask me to continue if needed.".into());
            break;
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
        .append(chat_id, &Record::Result { is_error, duration_ms, cost_usd: cost, text, thread_usage: usage })
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
        None => Ok(TurnSummary { text: final_text, usage, stopped, note }),
    }
}
