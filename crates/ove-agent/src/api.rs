//! A small Anthropic Messages API client over raw HTTP (Rust has no official SDK): streaming Server-Sent Events,
//! retries with backoff on rate limits and overload, and plain-language errors.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};

/// API version header value.
pub const API_VERSION: &str = "2026-10-21";
/// Default API host. `OVE_ANTHROPIC_BASE_URL` overrides it for tests and local mocks, in debug builds only (or with
/// `OVE_DEV_ENDPOINTS=1`), and only with an https URL or http to localhost; see [`ove_guard::endpoint`].
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Environment variable that overrides [`DEFAULT_BASE_URL`].
pub const BASE_URL_VAR: &str = "OVE_ANTHROPIC_BASE_URL";
/// Beta header enabling server-side refusal fallbacks.
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Models the user can pick. The first is the default.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "claude-sonnet-5-5",
        label: "Claude Sonnet 5.5",
        input_per_mtok: 2.0,
        output_per_mtok: 10.0,
        cache_read_per_mtok: 0.10,
        context_window: 1_000_000,
        fallbacks: true,
    },
    ModelInfo {
        id: "claude-opus-5-5",
        label: "Claude Opus 5.5",
        input_per_mtok: 4.0,
        output_per_mtok: 20.0,
        cache_read_per_mtok: 0.20,
        context_window: 1_000_000,
        fallbacks: true,
    },
    ModelInfo {
        id: "claude-haiku-5-5",
        label: "Claude Haiku 5.5",
        input_per_mtok: 0.10,
        output_per_mtok: 0.50,
        cache_read_per_mtok: 0.01,
        context_window: 1_000_000,
        fallbacks: false,
    },
];

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    /// Input context window in tokens.
    pub context_window: u64,
    /// Server-side refusal fallbacks are available for this model.
    pub fallbacks: bool,
}

pub fn model_info(id: &str) -> ModelInfo {
    known_model(id).unwrap_or(MODELS[0])
}

/// The listed model `id` names: exact, or a dated / suffixed variant of a listed id (`claude-opus-5-5-20261101`).
pub fn known_model(id: &str) -> Option<ModelInfo> {
    MODELS.iter().copied().find(|m| m.id == id).or_else(|| {
        MODELS
            .iter()
            .copied()
            .filter(|m| id.starts_with(m.id) && id[m.id.len()..].starts_with('-'))
            .max_by_key(|m| m.id.len())
    })
}

/// The model to price a response by: the one the API reports it was served by (a server-side fallback can differ
/// from the one asked for), or `requested` when the reported one is missing or not in [`MODELS`].
pub fn pricing_model<'a>(reported: &'a str, requested: &'a str) -> &'a str {
    if known_model(reported).is_some() { reported } else { requested }
}

/// Token usage of one or more requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_input_tokens += o.cache_read_input_tokens;
        self.cache_creation_input_tokens += o.cache_creation_input_tokens;
    }

    /// Estimated cost in US dollars (cache writes at 1.25× input).
    pub fn cost_usd(&self, model: &str) -> f64 {
        let m = model_info(model);
        (self.input_tokens as f64 * m.input_per_mtok
            + self.cache_creation_input_tokens as f64 * m.input_per_mtok * 1.25
            + self.cache_read_input_tokens as f64 * m.cache_read_per_mtok
            + self.output_tokens as f64 * m.output_per_mtok)
            / 1e6
    }

    fn merge_from(&mut self, v: &Value) {
        let g = |k: &str| v.get(k).and_then(Value::as_u64);
        if let Some(x) = g("input_tokens") {
            self.input_tokens = x;
        }
        if let Some(x) = g("output_tokens") {
            self.output_tokens = x;
        }
        if let Some(x) = g("cache_read_input_tokens") {
            self.cache_read_input_tokens = x;
        }
        if let Some(x) = g("cache_creation_input_tokens") {
            self.cache_creation_input_tokens = x;
        }
    }
}

/// An API failure, with a stable code and a message for the user.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    /// Worth retrying (rate limit, overload, server error, network).
    pub retryable: bool,
    /// Server-suggested wait.
    pub retry_after: Option<Duration>,
}

impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ApiError { code, message: message.into(), retryable: false, retry_after: None }
    }

    fn from_status(status: u16, body: &str, retry_after: Option<Duration>) -> Self {
        let detail = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| body.chars().take(200).collect());
        let (code, message, retryable) = match status {
            401 => (
                "auth.invalidKey",
                "The Anthropic API key was rejected. Check the key in the assistant settings.".to_string(),
                false,
            ),
            403 => ("auth.forbidden", format!("This API key isn't allowed to do that: {detail}"), false),
            404 => ("api.notFound", format!("The model or endpoint wasn't found: {detail}"), false),
            413 => {
                ("api.tooLarge", "The conversation is too large for one request. Start a new chat.".to_string(), false)
            }
            429 => ("api.rateLimited", "Rate limited by the Anthropic API. Retrying shortly…".to_string(), true),
            529 => ("api.overloaded", "The Anthropic API is overloaded right now. Retrying…".to_string(), true),
            500..=599 => ("api.server", format!("The Anthropic API had a server error ({status})."), true),
            _ => ("api.badRequest", format!("The request was rejected: {detail}"), false),
        };
        ApiError { code, message, retryable, retry_after }
    }

    fn network(e: impl std::fmt::Display) -> Self {
        ApiError {
            code: "net.unreachable",
            message: format!("Couldn't reach the Anthropic API. Check your internet connection. ({e})"),
            retryable: true,
            retry_after: None,
        }
    }

    pub fn cancelled() -> Self {
        ApiError::new("turn.cancelled", "Stopped.")
    }
}

/// What the client reports while a response streams.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// A tool call began (id, name).
    ToolUseStart(String, String),
    /// Tokens billed for one response (attempt), with the model the API reports serving it. Sent once per attempt
    /// that got as far as `message_start`, also when the stream then fails or is stopped, so partial responses are
    /// counted. Not output: it does not stop a retry.
    Usage {
        model: String,
        usage: Usage,
    },
}

/// One complete model response.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub id: String,
    pub model: String,
    /// Content blocks exactly as the API sent them (thinking, fallback and unknown blocks included), to be passed
    /// back unchanged in the next request.
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
    /// `tool_use` block ids whose streamed input was not valid JSON (the raw text is kept for the error result).
    pub invalid_tool_inputs: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    api_key: String,
    /// Delays before retries 1, 2, 3, …
    pub backoff: Vec<Duration>,
}

impl Client {
    /// A client for the real API, or for the override in `OVE_ANTHROPIC_BASE_URL` where the build allows it.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, ove_guard::endpoint::base_url(BASE_URL_VAR, DEFAULT_BASE_URL))
    }

    /// The API root requests go to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// A client for an explicit API root (tests, embedding). Unlike [`Client::new`] this trusts the caller.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        let api_key = api_key.into();
        ove_guard::register_secret(&api_key);
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            // SSE sends pings; a long silence means the connection is gone.
            .timeout_read(Duration::from_secs(120))
            // ureq forwards `x-api-key` on a redirect to another host (it only drops `Authorization`); the API never
            // redirects, so redirects are not followed at all.
            .redirects(0)
            .build();
        Client {
            agent,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            backoff: vec![
                Duration::from_secs(1),
                Duration::from_secs(3),
                Duration::from_secs(8),
                Duration::from_secs(20),
            ],
        }
    }

    /// Streams one Messages request. Retries before any output arrives on retryable failures; `cancel` stops it.
    pub fn stream(
        &self,
        body: &Value,
        cancel: &AtomicBool,
        on_event: &mut dyn FnMut(StreamEvent),
        on_retry: &mut dyn FnMut(&ApiError, Duration),
    ) -> Result<Response, ApiError> {
        let mut attempt = 0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(ApiError::cancelled());
            }
            let mut produced = false;
            let result = self.stream_once(body, cancel, &mut |e| {
                if !matches!(e, StreamEvent::Usage { .. }) {
                    produced = true;
                }
                on_event(e)
            });
            match result {
                Err(e) if e.retryable && !produced && attempt < self.backoff.len() => {
                    let wait = e.retry_after.unwrap_or(self.backoff[attempt]).min(Duration::from_secs(60));
                    tracing::warn!(code = e.code, attempt, "retrying the Messages request");
                    on_retry(&e, wait);
                    let until = std::time::Instant::now() + wait;
                    while std::time::Instant::now() < until {
                        if cancel.load(Ordering::SeqCst) {
                            return Err(ApiError::cancelled());
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// Counts the input tokens of a Messages request body (`POST /v1/messages/count_tokens`; free, nothing is
    /// generated). One attempt, no retries.
    pub fn count_tokens(&self, body: &Value) -> Result<u64, ApiError> {
        let req = self
            .agent
            .post(&format!("{}/v1/messages/count_tokens", self.base_url))
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", API_VERSION)
            .set("content-type", "application/json");
        let resp = match req.send_string(&body.to_string()) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(ApiError::from_status(code, &text, None));
            }
            Err(ureq::Error::Transport(t)) => return Err(ApiError::network(t)),
        };
        let text = resp.into_string().map_err(ApiError::network)?;
        serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["input_tokens"].as_u64())
            .ok_or_else(|| ApiError::new("api.badResponse", "Unreadable response from the API."))
    }

    /// Checks that the key works with the smallest request there is: counting the tokens of a one-word message.
    /// Costs nothing and generates nothing.
    pub fn check_key(&self, model: &str) -> Result<(), ApiError> {
        match self.count_tokens(&json!({"model": model, "messages": [{"role": "user", "content": "hi"}]})) {
            // Rate limited: the key was accepted.
            Err(e) if e.code == "api.rateLimited" => Ok(()),
            r => r.map(|_| ()),
        }
    }

    fn stream_once(
        &self,
        body: &Value,
        cancel: &AtomicBool,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Response, ApiError> {
        let mut req = self
            .agent
            .post(&format!("{}/v1/messages", self.base_url))
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", API_VERSION)
            .set("content-type", "application/json")
            .set("accept", "text/event-stream");
        if body.get("fallbacks").is_some() {
            req = req.set("anthropic-beta", FALLBACK_BETA);
        }
        let resp = match req.send_string(&body.to_string()) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let retry_after =
                    r.header("retry-after").and_then(|s| s.trim().parse::<u64>().ok()).map(Duration::from_secs);
                let text = r.into_string().unwrap_or_default();
                return Err(ApiError::from_status(code, &text, retry_after));
            }
            Err(ureq::Error::Transport(t)) => return Err(ApiError::network(t)),
        };
        parse_sse(BufReader::new(resp.into_reader()), cancel, on_event)
    }
}

/// Parses a Messages SSE stream into a complete response. Once the stream has started (`message_start`), its usage
/// is reported with [`StreamEvent::Usage`] whether it then completes, fails or is cancelled.
pub fn parse_sse(
    reader: impl BufRead,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<Response, ApiError> {
    let mut out = Response {
        id: String::new(),
        model: String::new(),
        content: Vec::new(),
        stop_reason: None,
        usage: Usage::default(),
        invalid_tool_inputs: Vec::new(),
    };
    let mut started = false;
    let res = read_sse(reader, cancel, on_event, &mut out, &mut started);
    if started {
        on_event(StreamEvent::Usage { model: out.model.clone(), usage: out.usage });
    }
    res?;
    out.content.retain(|b| !b.is_null());
    Ok(out)
}

fn read_sse(
    reader: impl BufRead,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(StreamEvent),
    out: &mut Response,
    started: &mut bool,
) -> Result<(), ApiError> {
    let mut partial_json: Vec<String> = Vec::new();
    let mut data = String::new();
    let mut done = false;
    for line in reader.lines() {
        if cancel.load(Ordering::SeqCst) {
            return Err(ApiError::cancelled());
        }
        let line = line.map_err(ApiError::network)?;
        if let Some(d) = line.strip_prefix("data:") {
            data.push_str(d.trim_start());
            continue;
        }
        if !line.is_empty() {
            continue; // `event:` lines and comments; the type is also inside `data`.
        }
        if data.is_empty() {
            continue;
        }
        let ev: Value = serde_json::from_str(&data)
            .map_err(|e| ApiError::new("api.badStream", format!("Unreadable response from the API: {e}")))?;
        data.clear();
        match ev["type"].as_str().unwrap_or("") {
            "message_start" => {
                *started = true;
                let m = &ev["message"];
                out.id = m["id"].as_str().unwrap_or_default().to_string();
                out.model = m["model"].as_str().unwrap_or_default().to_string();
                out.usage.merge_from(&m["usage"]);
            }
            "content_block_start" => {
                let i = ev["index"].as_u64().unwrap_or(0) as usize;
                let block = ev["content_block"].clone();
                if block["type"] == "tool_use" {
                    on_event(StreamEvent::ToolUseStart(
                        block["id"].as_str().unwrap_or_default().into(),
                        block["name"].as_str().unwrap_or_default().into(),
                    ));
                }
                while out.content.len() <= i {
                    out.content.push(Value::Null);
                    partial_json.push(String::new());
                }
                out.content[i] = block;
            }
            "content_block_delta" => {
                let i = ev["index"].as_u64().unwrap_or(0) as usize;
                let Some(block) = out.content.get_mut(i) else { continue };
                let d = &ev["delta"];
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or_default();
                        append(block, "text", t);
                        on_event(StreamEvent::TextDelta(t.to_string()));
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        append(block, "thinking", t);
                        on_event(StreamEvent::ThinkingDelta(t.to_string()));
                    }
                    "signature_delta" => {
                        if let Some(o) = block.as_object_mut() {
                            o.insert("signature".into(), d["signature"].clone());
                        }
                    }
                    "input_json_delta" => partial_json[i].push_str(d["partial_json"].as_str().unwrap_or_default()),
                    _ => {}
                }
            }
            "content_block_stop" => {
                let i = ev["index"].as_u64().unwrap_or(0) as usize;
                if let Some(block) = out.content.get_mut(i)
                    && block["type"] == "tool_use"
                {
                    let raw = std::mem::take(&mut partial_json[i]);
                    let parsed = if raw.trim().is_empty() {
                        Ok(Value::Object(Map::new()))
                    } else {
                        serde_json::from_str::<Value>(&raw)
                    };
                    match parsed {
                        Ok(v) if v.is_object() => {
                            block["input"] = v;
                        }
                        _ => {
                            let id = block["id"].as_str().unwrap_or_default().to_string();
                            out.invalid_tool_inputs.push((id, raw));
                            block["input"] = json!({});
                        }
                    }
                }
            }
            "message_delta" => {
                if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                    out.stop_reason = Some(r.to_string());
                }
                let mut u = out.usage;
                u.merge_from(&ev["usage"]);
                out.usage = u;
            }
            "message_stop" => {
                done = true;
                break;
            }
            "error" => {
                let kind = ev["error"]["type"].as_str().unwrap_or("error");
                let msg = ev["error"]["message"].as_str().unwrap_or("stream error").to_string();
                let status = match kind {
                    "overloaded_error" => 529,
                    "rate_limit_error" => 429,
                    "api_error" => 500,
                    _ => 400,
                };
                return Err(ApiError::from_status(status, &json!({"error": {"message": msg}}).to_string(), None));
            }
            _ => {} // ping and future event types
        }
    }
    if !done {
        return Err(ApiError {
            code: "net.interrupted",
            message: "The connection closed before the response finished.".into(),
            retryable: true,
            retry_after: None,
        });
    }
    Ok(())
}

fn append(block: &mut Value, key: &str, text: &str) {
    if let Some(o) = block.as_object_mut() {
        let cur = o.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        o.insert(key.into(), Value::String(cur + text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(events: &[Value]) -> String {
        events.iter().map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e)).collect()
    }

    #[test]
    fn parses_text_thinking_and_tool_use() {
        let body = sse(&[
            json!({"type":"message_start","message":{"id":"msg_1","model":"claude-sonnet-5-5","usage":{"input_tokens":10,"cache_read_input_tokens":5}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hel"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"lo"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"ping"}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"tu_1","name":"print_timeline","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"from\": "}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"1}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}),
            json!({"type":"message_stop"}),
        ]);
        let mut seen = vec![];
        let r = parse_sse(body.as_bytes(), &AtomicBool::new(false), &mut |e| seen.push(e)).unwrap();
        assert_eq!(r.content.len(), 3);
        assert_eq!(r.content[0], json!({"type":"thinking","thinking":"hmm","signature":"sig"}));
        assert_eq!(r.content[1]["text"], "Hello");
        assert_eq!(r.content[2]["input"], json!({"from": 1}));
        assert_eq!(r.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(
            r.usage,
            Usage { input_tokens: 10, output_tokens: 42, cache_read_input_tokens: 5, cache_creation_input_tokens: 0 }
        );
        assert!(seen.contains(&StreamEvent::TextDelta("lo".into())));
        assert!(seen.contains(&StreamEvent::ToolUseStart("tu_1".into(), "print_timeline".into())));
    }

    #[test]
    fn invalid_tool_json_is_reported_not_run() {
        let body = sse(&[
            json!({"type":"message_start","message":{"id":"m","model":"x","usage":{}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu","name":"apply_timeline","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"edits\": [ {\"oldText\": \"a"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ]);
        let r = parse_sse(body.as_bytes(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r.invalid_tool_inputs.len(), 1);
        assert_eq!(r.invalid_tool_inputs[0].0, "tu");
    }

    #[test]
    fn stream_error_and_truncation() {
        let body = sse(&[json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}})]);
        let e = parse_sse(body.as_bytes(), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert_eq!(e.code, "api.overloaded");
        assert!(e.retryable);
        let cut = sse(&[json!({"type":"message_start","message":{"id":"m","model":"x","usage":{}}})]);
        assert_eq!(
            parse_sse(cut.as_bytes(), &AtomicBool::new(false), &mut |_| {}).unwrap_err().code,
            "net.interrupted"
        );
    }

    #[test]
    fn usage_is_reported_even_when_the_stream_fails() {
        let usage_of = |body: String| {
            let mut got = vec![];
            let r = parse_sse(body.as_bytes(), &AtomicBool::new(false), &mut |e| {
                if let StreamEvent::Usage { model, usage } = e {
                    got.push((model, usage));
                }
            });
            (r.is_ok(), got)
        };
        let start =
            json!({"type":"message_start","message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":50}}});
        let delta = json!({"type":"message_delta","delta":{},"usage":{"output_tokens":7}});
        let err = json!({"type":"error","error":{"type":"api_error","message":"boom"}});
        let (ok, got) = usage_of(sse(&[start.clone(), delta.clone(), err.clone()]));
        assert!(!ok);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "claude-opus-5-5");
        assert_eq!((got[0].1.input_tokens, got[0].1.output_tokens), (50, 7));
        // Cut off, and complete: one report each.
        assert_eq!(usage_of(sse(std::slice::from_ref(&start))).1.len(), 1);
        assert_eq!(usage_of(sse(&[start, delta, json!({"type":"message_stop"})])).1.len(), 1);
        // Failed before the response started: nothing was billed.
        assert!(usage_of(sse(&[err])).1.is_empty());
    }

    #[test]
    fn pricing_follows_the_served_model() {
        assert_eq!(known_model("claude-haiku-5-5-20261001").unwrap().id, "claude-haiku-5-5");
        assert!(known_model("claude-haiku-5-50").is_none());
        assert!(known_model("mock").is_none());
        assert_eq!(pricing_model("claude-opus-5-5", "claude-sonnet-5-5"), "claude-opus-5-5");
        assert_eq!(pricing_model("", "claude-sonnet-5-5"), "claude-sonnet-5-5");
        assert_eq!(pricing_model("mock", "claude-haiku-5-5"), "claude-haiku-5-5");
    }

    #[test]
    fn base_url_override_must_be_https_or_local() {
        // The only test in this crate that touches the variable.
        let with = |v: &str| {
            unsafe { std::env::set_var(BASE_URL_VAR, v) };
            let c = Client::new("sk-ant-test-0123456789abcdef");
            unsafe { std::env::remove_var(BASE_URL_VAR) };
            c.base_url().to_string()
        };
        // Debug build (tests): local mocks and https hosts are accepted…
        assert_eq!(with("http://127.0.0.1:8787/"), "http://127.0.0.1:8787");
        assert_eq!(with("https://proxy.example.com"), "https://proxy.example.com");
        // …plain http to another machine is not.
        assert_eq!(with("http://203.0.113.9"), DEFAULT_BASE_URL);
        assert_eq!(with(""), DEFAULT_BASE_URL);
    }

    #[test]
    fn status_mapping_and_cost() {
        assert_eq!(ApiError::from_status(401, "{}", None).code, "auth.invalidKey");
        assert!(ApiError::from_status(429, "{}", None).retryable);
        assert!(!ApiError::from_status(400, r#"{"error":{"message":"bad"}}"#, None).retryable);
        assert!(ApiError::from_status(400, r#"{"error":{"message":"bad"}}"#, None).message.contains("bad"));
        let u = Usage { input_tokens: 1_000_000, output_tokens: 1_000_000, ..Default::default() };
        assert!((u.cost_usd("claude-sonnet-5-5") - 12.0).abs() < 1e-9);
    }
}
