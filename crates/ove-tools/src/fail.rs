//! Refusals: a stable code, a message for people and agents, and optional details for JSON.

use ove_projection::Refusal;
use ove_store::StoreError;
use serde_json::{Value, json};

/// A refused or failed tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct Fail {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl Fail {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Fail {
        Fail { code: code.into(), message: message.into(), details: Value::Null }
    }

    pub fn details(mut self, v: Value) -> Fail {
        self.details = v;
        self
    }

    /// The JSON error object (`{"error": {code, message, details}}`).
    pub fn to_json(&self) -> Value {
        json!({"error": {"code": self.code, "message": self.message, "details": self.details}})
    }

    /// The error as text for a model: the message, then the code in brackets.
    pub fn model_text(&self) -> String {
        format!("{} ({})", self.message, self.code)
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<StoreError> for Fail {
    fn from(e: StoreError) -> Self {
        Fail::new(e.code(), e.to_string())
    }
}

impl From<Refusal> for Fail {
    fn from(r: Refusal) -> Self {
        Fail { code: r.code.clone(), message: r.to_string(), details: json!({"diagnostics": r.diagnostics}) }
    }
}

impl From<ove_control::CallError> for Fail {
    fn from(e: ove_control::CallError) -> Self {
        let message = match &e {
            ove_control::CallError::NotRunning => {
                "the app is not running (start OpenVideoEditor, or use a command that works on files)".to_string()
            }
            other => other.to_string(),
        };
        Fail::new(e.code(), message)
    }
}

impl From<ove_control::Refusal> for Fail {
    fn from(r: ove_control::Refusal) -> Self {
        Fail::new(r.code, r.message)
    }
}

/// `"n word"` with a plural `s` when n ≠ 1.
pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// Seconds → µs, refusing negatives and non-numbers.
pub fn secs(s: f64, what: &str) -> Result<i64, Fail> {
    ove_core::time::secs_to_micros(s)
        .filter(|v| *v >= 0)
        .ok_or_else(|| Fail::new("argument.invalid", format!("{what} must be a number of seconds ≥ 0")))
}

pub fn now_ms() -> i64 {
    ove_core::Clock::now_ms(&ove_core::SystemClock)
}
