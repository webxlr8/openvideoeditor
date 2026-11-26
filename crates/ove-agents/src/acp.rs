//! The app's side of the Agent Client Protocol: it starts an installed agent as a subprocess and talks JSON-RPC 2.0
//! with it, one message per line on the agent's stdin and stdout (<https://agentclientprotocol.com>, protocol
//! version 1).
//!
//! The client is where the harmless-agent rules that do not depend on the agent are enforced
//! (add-local-agents-and-onboarding design D6):
//! - layer 2: the file-system and terminal capabilities are not offered; an agent that calls them anyway gets
//!   "method not found", and a permission request is granted only for one of the app's own MCP tools;
//! - layer 4: the agent runs in an empty folder made for the session;
//! - layer 6: a turn has a time limit, and [`Session::close`] (Stop) ends the agent's process.
//!
//! Every refusal is reported as [`Event::Refused`] and logged.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The protocol version this client speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// The `ove mcp` server the session gets (an absolute program path and its arguments).
#[derive(Debug, Clone)]
pub struct McpServer {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
}

/// How to start the agent.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Extra environment for the agent (added to the app's own).
    pub env: Vec<(String, String)>,
}

/// What happens during a turn, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Part of the agent's answer.
    Text(String),
    /// Part of the agent's reasoning, when it shares it.
    Thought(String),
    /// The agent started a tool call.
    ToolStarted { id: String, title: String, kind: String },
    /// A tool call changed status (`pending`, `in_progress`, `completed`, `failed`).
    ToolStatus { id: String, status: String },
    /// The agent's plan (each entry's text).
    Plan(Vec<String>),
    /// Something the agent asked for and was not allowed; `what` says what, for the person and the log.
    Refused { what: String },
}

/// Why a session or turn failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AcpError {
    #[error("the agent could not be started: {0}")]
    Start(String),
    /// The agent needs its person to sign in first (`-32000`, or a start that failed for that reason).
    #[error("{message}")]
    SignIn { message: String },
    #[error("the agent stopped unexpectedly{}", stderr_tail(.stderr))]
    Exited { stderr: String },
    #[error("the agent did not answer within {0} s")]
    Timeout(u64),
    #[error("the agent reported an error: {0}")]
    Agent(String),
    #[error("the agent speaks protocol version {0}, this app version {PROTOCOL_VERSION}")]
    Version(u64),
}

fn stderr_tail(s: &str) -> String {
    let t: String =
        s.trim().lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" / ");
    if t.is_empty() { String::new() } else { format!(": {}", t.chars().take(400).collect::<String>()) }
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// `end_turn`, `max_tokens`, `max_turn_requests`, `refusal` or another value the agent reported.
    Reason(String),
    /// The turn was cancelled (Stop pressed).
    Cancelled,
}

/// Which tool calls may be granted: those whose title or name mentions the app's MCP server or is one of its tools.
#[derive(Debug, Clone)]
pub struct Allow {
    pub server: String,
    pub tools: Vec<String>,
}

impl Allow {
    /// True when a permission request is for one of the app's own tools.
    pub fn permits(&self, tool_call: &Value) -> bool {
        let field = |k: &str| tool_call.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let kind = field("kind");
        // Kinds that are never one of the app's tools, whatever the title says.
        if matches!(kind.as_str(), "execute" | "delete" | "move" | "fetch") {
            return false;
        }
        [field("title"), field("name")].iter().any(|t| {
            let t = t.trim();
            if t.is_empty() {
                return false;
            }
            // Agents name MCP tools `mcp__<server>__<tool>`, `<server>__<tool>`, `<server>/<tool>` or `<tool>`.
            let bare = t.rsplit(['_', '/', '.', ':']).next().unwrap_or(t);
            let qualified = t.contains(&self.server) && self.tools.iter().any(|n| t.ends_with(n.as_str()));
            qualified || self.tools.iter().any(|n| n == t || (n == bare && t.ends_with(&format!("__{n}"))))
        })
    }
}

enum Incoming {
    Message(Value),
    Closed,
}

/// A running agent with one session.
pub struct Session {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Incoming>,
    /// Messages read while waiting for something else, handled next.
    queue: VecDeque<Value>,
    stderr: std::sync::Arc<std::sync::Mutex<String>>,
    next_id: i64,
    pub session_id: String,
    pub agent_name: String,
    allow: Allow,
    /// The session's working folder (empty; removed with the session).
    _cwd: tempfile::TempDir,
}

/// Kept from the agent's stderr, for error messages.
const STDERR_KEEP: usize = 16 * 1024;

fn spawn(launch: &Launch, cwd: &Path) -> Result<Child, AcpError> {
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: the agent is a console program; no window may flash over the app.
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map_err(|e| AcpError::Start(format!("{}: {e}", launch.program.display())))
}

impl Session {
    /// Starts the agent, initialises the protocol and opens a session in a new empty folder with `server` as its
    /// only MCP server. `limit` bounds each step of the start.
    pub fn start(launch: &Launch, server: &McpServer, allow: Allow, limit: Duration) -> Result<Session, AcpError> {
        let cwd = tempfile::Builder::new()
            .prefix("ove-agent-")
            .tempdir()
            .map_err(|e| AcpError::Start(format!("session folder: {e}")))?;
        let mut child = spawn(launch, cwd.path())?;
        let stdin = child.stdin.take().ok_or_else(|| AcpError::Start("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| AcpError::Start("no stdout".into()))?;
        let stderr_pipe = child.stderr.take();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("acp-read".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(&line) {
                        Ok(v) => {
                            if tx.send(Incoming::Message(v)).is_err() {
                                return;
                            }
                        }
                        Err(_) => tracing::debug!(line = %line.chars().take(200).collect::<String>(), "agent wrote a line that is not JSON"),
                    }
                }
                let _ = tx.send(Incoming::Closed);
            })
            .map_err(|e| AcpError::Start(format!("reader thread: {e}")))?;
        let stderr = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        if let Some(pipe) = stderr_pipe {
            let keep = stderr.clone();
            let _ = std::thread::Builder::new().name("acp-stderr".into()).spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    tracing::debug!(target: "agent", "{line}");
                    if let Ok(mut k) = keep.lock()
                        && k.len() < STDERR_KEEP
                    {
                        k.push_str(&line);
                        k.push('\n');
                    }
                }
            });
        }
        let mut s = Session {
            child,
            stdin,
            rx,
            queue: VecDeque::new(),
            stderr,
            next_id: 0,
            session_id: String::new(),
            agent_name: String::new(),
            allow,
            _cwd: cwd,
        };
        let init = s.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                // Layer 2: no file system and no terminal for the agent.
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "openvideoeditor", "title": "OpenVideoEditor", "version": env!("CARGO_PKG_VERSION")},
            }),
            limit,
            &AtomicBool::new(false),
            &mut |_| {},
        )?;
        let version = init.get("protocolVersion").and_then(Value::as_u64).unwrap_or(PROTOCOL_VERSION);
        if version != PROTOCOL_VERSION {
            return Err(AcpError::Version(version));
        }
        s.agent_name = init
            .pointer("/agentInfo/title")
            .or_else(|| init.pointer("/agentInfo/name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cwd = s._cwd.path().to_path_buf();
        let new = s.request(
            "session/new",
            json!({
                "cwd": cwd,
                "mcpServers": [{
                    "name": server.name,
                    "command": server.command,
                    "args": server.args,
                    "env": [],
                }],
            }),
            limit,
            &AtomicBool::new(false),
            &mut |_| {},
        )?;
        s.session_id = new
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| AcpError::Agent("session/new gave no sessionId".into()))?
            .to_string();
        Ok(s)
    }

    fn write(&mut self, v: &Value) -> Result<(), AcpError> {
        writeln!(self.stdin, "{v}").and_then(|()| self.stdin.flush()).map_err(|_| self.exited())
    }

    fn exited(&mut self) -> AcpError {
        // Give the stderr reader a moment to collect the last lines.
        std::thread::sleep(Duration::from_millis(100));
        let _ = self.child.try_wait();
        AcpError::Exited { stderr: self.stderr.lock().map(|s| s.clone()).unwrap_or_default() }
    }

    /// Sends one request and waits for its response, handling everything the agent sends meanwhile.
    fn request(
        &mut self,
        method: &str,
        params: Value,
        limit: Duration,
        cancel: &AtomicBool,
        on: &mut dyn FnMut(Event),
    ) -> Result<Value, AcpError> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + limit;
        let mut cancel_sent = false;
        loop {
            if cancel.load(Ordering::SeqCst) && !cancel_sent && method == "session/prompt" {
                cancel_sent = true;
                let sid = self.session_id.clone();
                self.write(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sid}}))?;
            }
            let msg = match self.queue.pop_front() {
                Some(m) => m,
                None => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(AcpError::Timeout(limit.as_secs()));
                    }
                    match self.rx.recv_timeout(left.min(Duration::from_millis(100))) {
                        Ok(Incoming::Message(m)) => m,
                        Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => return Err(self.exited()),
                        Err(RecvTimeoutError::Timeout) => continue,
                    }
                }
            };
            let has_method = msg.get("method").and_then(Value::as_str).map(str::to_string);
            match (msg.get("id").cloned(), has_method) {
                // Our response.
                (Some(mid), None) if mid.as_i64() == Some(id) => {
                    if let Some(e) = msg.get("error") {
                        let code = e.get("code").and_then(Value::as_i64).unwrap_or(0);
                        let mut message = e.get("message").and_then(Value::as_str).unwrap_or("error").to_string();
                        // Agents put the useful part in `data` ("Internal error" alone says nothing).
                        let details =
                            e.pointer("/data/details").and_then(Value::as_str).map(str::to_string).or_else(|| {
                                e.get("data")
                                    .filter(|d| !d.is_null())
                                    .map(|d| d.as_str().map_or(d.to_string(), str::to_string))
                            });
                        if let Some(d) = details.filter(|d| !d.trim().is_empty()) {
                            message = format!("{message}: {}", d.trim().chars().take(600).collect::<String>());
                        }
                        return Err(if code == -32000 || looks_like_sign_in(&message) {
                            AcpError::SignIn { message }
                        } else {
                            AcpError::Agent(message)
                        });
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                // A request from the agent.
                (Some(rid), Some(m)) => {
                    let params = msg.get("params").cloned().unwrap_or(Value::Null);
                    let reply = self.answer(&m, &params, on);
                    let out = match reply {
                        Ok(result) => json!({"jsonrpc": "2.0", "id": rid, "result": result}),
                        Err((code, message)) => {
                            json!({"jsonrpc": "2.0", "id": rid, "error": {"code": code, "message": message}})
                        }
                    };
                    self.write(&out)?;
                }
                // A notification.
                (None, Some(m)) if m == "session/update" => {
                    if let Some(u) = msg.pointer("/params/update") {
                        update(u, on);
                    }
                }
                _ => {}
            }
        }
    }

    /// The answer to a request from the agent.
    fn answer(&mut self, method: &str, params: &Value, on: &mut dyn FnMut(Event)) -> Result<Value, (i64, String)> {
        match method {
            "session/request_permission" => {
                let tool = params.get("toolCall").cloned().unwrap_or(Value::Null);
                let options = params.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
                let pick = |kinds: &[&str]| {
                    options
                        .iter()
                        .find(|o| o.get("kind").and_then(Value::as_str).is_some_and(|k| kinds.contains(&k)))
                        .and_then(|o| o.get("optionId").cloned())
                };
                if self.allow.permits(&tool)
                    && let Some(id) = pick(&["allow_once", "allow_always"])
                {
                    return Ok(json!({"outcome": {"outcome": "selected", "optionId": id}}));
                }
                let what = describe(&tool);
                tracing::warn!(what = %what, "refused an agent's permission request");
                on(Event::Refused { what });
                Ok(match pick(&["reject_once", "reject_always"]) {
                    Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
                    None => json!({"outcome": {"outcome": "cancelled"}}),
                })
            }
            m if m.starts_with("fs/") || m.starts_with("terminal/") => {
                let what = match m {
                    "fs/read_text_file" => {
                        format!("read the file {}", params.get("path").and_then(Value::as_str).unwrap_or("?"))
                    }
                    "fs/write_text_file" => {
                        format!("write the file {}", params.get("path").and_then(Value::as_str).unwrap_or("?"))
                    }
                    "terminal/create" => {
                        format!("run the command {}", params.get("command").and_then(Value::as_str).unwrap_or("?"))
                    }
                    other => other.to_string(),
                };
                tracing::warn!(method = m, what = %what, "refused an agent's request for a capability it was not given");
                on(Event::Refused { what });
                Err((-32601, format!("{m} is not available: this client offers no file system or terminal")))
            }
            other => Err((-32601, format!("method not found: {other}"))),
        }
    }

    /// Sends one prompt and streams the turn's events until it ends. Setting `cancel` sends `session/cancel` and
    /// waits for the agent to stop (or `limit`).
    pub fn prompt(
        &mut self,
        text: &str,
        limit: Duration,
        cancel: &AtomicBool,
        on: &mut dyn FnMut(Event),
    ) -> Result<Stop, AcpError> {
        let sid = self.session_id.clone();
        let r = self.request(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
            limit,
            cancel,
            on,
        );
        match r {
            Ok(v) => {
                let reason = v.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_string();
                Ok(if reason == "cancelled" { Stop::Cancelled } else { Stop::Reason(reason) })
            }
            Err(AcpError::Timeout(s)) => {
                // Layer 6: past the limit the turn is over, whatever the agent is doing.
                self.close();
                Err(AcpError::Timeout(s))
            }
            Err(e) => Err(e),
        }
    }

    /// Ends the agent's process (Stop, or the end of the session).
    pub fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// What the agent wrote to stderr so far (bounded).
    pub fn stderr(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

/// A short description of a tool call, for the person and the log.
fn describe(tool: &Value) -> String {
    let title = tool.get("title").and_then(Value::as_str).unwrap_or("");
    let kind = tool.get("kind").and_then(Value::as_str).unwrap_or("");
    let input = tool.get("rawInput").map(|v| v.to_string()).unwrap_or_default();
    let mut s = match (title.is_empty(), kind.is_empty()) {
        (false, false) => format!("{title} ({kind})"),
        (false, true) => title.to_string(),
        (true, false) => kind.to_string(),
        (true, true) => "a tool".to_string(),
    };
    if !input.is_empty() && input != "null" {
        s.push_str(&format!(": {}", input.chars().take(160).collect::<String>()));
    }
    s
}

fn looks_like_sign_in(message: &str) -> bool {
    let m = message.to_lowercase();
    ["auth", "sign in", "signin", "log in", "login", "logged in", "api key", "credentials"]
        .iter()
        .any(|w| m.contains(w))
}

fn text_of(block: &Value) -> Option<String> {
    (block.get("type").and_then(Value::as_str) == Some("text"))
        .then(|| block.get("text").and_then(Value::as_str).unwrap_or("").to_string())
}

/// Maps one `session/update` to events.
fn update(u: &Value, on: &mut dyn FnMut(Event)) {
    let s = |k: &str| u.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("") {
        "agent_message_chunk" => {
            if let Some(t) = u.get("content").and_then(text_of) {
                on(Event::Text(t));
            }
        }
        "agent_thought_chunk" => {
            if let Some(t) = u.get("content").and_then(text_of) {
                on(Event::Thought(t));
            }
        }
        "tool_call" => {
            on(Event::ToolStarted { id: s("toolCallId"), title: s("title"), kind: s("kind") });
            let status = s("status");
            if !status.is_empty() && status != "pending" {
                on(Event::ToolStatus { id: s("toolCallId"), status });
            }
        }
        "tool_call_update" => {
            let status = s("status");
            if !status.is_empty() {
                on(Event::ToolStatus { id: s("toolCallId"), status });
            }
        }
        "plan" => {
            let entries = u.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
            on(Event::Plan(
                entries.iter().filter_map(|e| e.get("content").and_then(Value::as_str)).map(str::to_string).collect(),
            ));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow() -> Allow {
        Allow { server: "openvideoeditor".into(), tools: vec!["apply_timeline".into(), "look".into()] }
    }

    #[test]
    fn only_the_apps_tools_are_permitted() {
        let a = allow();
        for title in ["mcp__openvideoeditor__apply_timeline", "openvideoeditor__look", "openvideoeditor/look", "look"] {
            assert!(a.permits(&json!({"title": title, "kind": "edit"})), "{title}");
        }
        for (title, kind) in [
            ("Bash", "execute"),
            ("mcp__openvideoeditor__apply_timeline", "execute"),
            ("Write", "edit"),
            ("Read /etc/passwd", "read"),
            ("WebFetch", "fetch"),
            ("mcp__other__apply_timeline_x", "other"),
            ("rm -rf look", "delete"),
            ("", "other"),
        ] {
            assert!(!a.permits(&json!({"title": title, "kind": kind})), "{title} ({kind})");
        }
    }

    #[test]
    fn updates_become_events() {
        let mut got = Vec::new();
        let mut on = |e| got.push(e);
        update(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Hi"}}), &mut on);
        update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "1", "title": "look", "kind": "read", "status": "in_progress"}),
            &mut on,
        );
        update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "1", "status": "completed"}), &mut on);
        update(
            &json!({"sessionUpdate": "plan", "entries": [{"content": "Cut", "priority": "high", "status": "pending"}]}),
            &mut on,
        );
        update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": []}), &mut on);
        assert_eq!(
            got,
            vec![
                Event::Text("Hi".into()),
                Event::ToolStarted { id: "1".into(), title: "look".into(), kind: "read".into() },
                Event::ToolStatus { id: "1".into(), status: "in_progress".into() },
                Event::ToolStatus { id: "1".into(), status: "completed".into() },
                Event::Plan(vec!["Cut".into()]),
            ]
        );
    }

    #[test]
    fn sign_in_is_recognised() {
        assert!(looks_like_sign_in("Authentication required"));
        assert!(looks_like_sign_in("Please run /login"));
        assert!(!looks_like_sign_in("model overloaded"));
    }
}
