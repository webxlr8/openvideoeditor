//! Spending approved by a person (change unify-agent-tool-layer, D7). An agent's generation request spends only after
//! a person approves its price:
//! - for `ove mcp` and the command line, through the control method `confirm_spend`: the app shows a price card and
//!   answers with the person's choice;
//! - for the in-app assistant, through its question card (`assistant.rs`).
//!
//! The approval is only ever the answer to the call that showed the card. Nothing records it for later use, so an
//! agent cannot approve its own quote by calling anything. The per-session spend cap is a setting kept in
//! `<config>/generation.json`, which `ove mcp` reads too.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker::relock;

/// Price cards waiting for the person, by quote id.
#[derive(Default)]
pub struct SpendCards {
    waiting: Mutex<HashMap<String, Sender<bool>>>,
}

/// A price card to show: an agent asks to spend.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct SpendRequested {
    pub quote_id: String,
    /// The question and the details, one per line.
    pub text: String,
    pub estimate_usd: f64,
    /// Who asks (the agent's client name, or its kind).
    pub client: String,
    pub prompt: String,
    pub project: String,
    /// Milliseconds since the epoch.
    pub expires_at: f64,
}

/// A price card was answered or ran out of time: the UI removes it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct SpendSettled(pub String);

fn refusal(code: &str, message: impl Into<String>) -> ove_control::Refusal {
    ove_control::Refusal::new(code, message)
}

/// The control method `confirm_spend`: shows the card and waits for the person (at most until the quote expires).
/// Answers `{quoteId, approved}`.
pub fn confirm_spend(app: &AppHandle, card: Value) -> Result<Value, ove_control::Refusal> {
    let quote = card["quoteId"]
        .as_str()
        .filter(|q| ove_guard::is_safe_id(q))
        .ok_or_else(|| refusal("argument.invalid", "confirm_spend needs the quote id"))?;
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
    let expires = card["expiresAt"].as_i64().unwrap_or(now);
    let left = (expires - now).clamp(0, ove_tools::session::QUOTE_TTL_MS);
    if left == 0 {
        return Err(refusal("quote.expired", "the quote expired before it could be shown"));
    }
    let (tx, rx) = channel();
    let cards = app.state::<SpendCards>();
    {
        let mut w = relock(&cards.waiting);
        if w.contains_key(quote) {
            return Err(refusal("confirm.pending", "this quote's card is already waiting for the person"));
        }
        w.insert(quote.to_string(), tx);
    }
    let request = SpendRequested {
        quote_id: quote.to_string(),
        text: ove_tools::tools::generate::card_text(&card),
        estimate_usd: card["estimateUsd"].as_f64().unwrap_or(0.0),
        client: card["actor"]["client"]
            .as_str()
            .or_else(|| card["actor"]["kind"].as_str())
            .unwrap_or("An agent")
            .into(),
        prompt: card["prompt"].as_str().unwrap_or_default().into(),
        project: card["project"]["name"].as_str().unwrap_or_default().into(),
        expires_at: expires as f64,
    };
    tracing::info!(quote, usd = request.estimate_usd, client = %request.client, "price card shown");
    let _ = request.emit(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    let approved = rx.recv_timeout(Duration::from_millis(left as u64)).unwrap_or(false);
    relock(&cards.waiting).remove(quote);
    let _ = SpendSettled(quote.to_string()).emit(app);
    tracing::info!(quote, approved, "price card answered");
    Ok(json!({"quoteId": quote, "approved": approved}))
}

/// The person's answer to a price card.
#[tauri::command]
#[specta::specta]
pub fn answer_spend(cards: State<'_, SpendCards>, quote_id: String, approved: bool) -> AppResult<()> {
    let tx = relock(&cards.waiting).remove(&quote_id);
    match tx {
        Some(tx) => tx
            .send(approved)
            .map_err(|_| AppError::new(ErrorCode::InvalidArgument, "That price card is no longer waiting.")),
        None => Err(AppError::new(ErrorCode::InvalidArgument, "That price card is no longer waiting.")),
    }
}

fn config() -> AppResult<std::path::PathBuf> {
    ove_tools::session::config_dir()
        .ok_or_else(|| AppError::new(ErrorCode::Io, "The app has no configuration folder on this computer."))
}

/// The most an agent session may spend on generation (USD).
#[tauri::command]
#[specta::specta]
pub fn spend_cap() -> f64 {
    ove_tools::session::spend_cap_usd(ove_tools::session::config_dir().as_deref())
}

/// Sets the per-session spend cap; returns it as saved (rounded to cents). Running sessions use it from their next
/// request.
#[tauri::command]
#[specta::specta]
pub fn set_spend_cap(usd: f64) -> AppResult<f64> {
    ove_tools::session::set_spend_cap_usd(&config()?, usd)
        .map_err(|f| AppError::new(ErrorCode::InvalidArgument, f.message))
}
