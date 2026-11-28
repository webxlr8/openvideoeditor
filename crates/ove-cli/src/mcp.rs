//! `ove mcp`: a Model Context Protocol server on stdio (JSON-RPC 2.0, one message per line), a thin adapter over the
//! tool registry (`ove-tools`): `tools/list` lists the registry's MCP tools and `tools/call` runs their handlers,
//! the same ones the command line runs. Edits are recorded with source `mcp`.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};

use ove_core::revision::{Actor, ActorKind, Source};
use ove_tools::{Fail, LinkCtx, Surface, ToolCtx};
use serde_json::{Map, Value, json};

/// Most images returned by one call.
const MAX_IMAGES: usize = 12;

/// The `tools/call` result object for a call.
pub fn call(ctx: &mut dyn ToolCtx, name: &str, args: &Map<String, Value>) -> Value {
    let spec = ove_tools::find(name);
    match ove_tools::call(ctx, name, args) {
        Ok(r) => {
            let text = spec.map(|s| s.model_text(&r)).unwrap_or_else(|| r.json.to_string());
            let mut content = vec![json!({"type": "text", "text": text})];
            for img in r.images.iter().take(MAX_IMAGES) {
                content.push(
                    json!({"type": "image", "data": ove_tools::receipt::base64(&img.bytes), "mimeType": "image/png"}),
                );
            }
            let structured = if r.json.is_object() { r.json } else { json!({"result": r.json}) };
            json!({"content": content, "structuredContent": structured, "isError": false})
        }
        Err(f) => {
            let e = f.to_json();
            json!({"content": [{"type": "text", "text": e.to_string()}], "structuredContent": e, "isError": true})
        }
    }
}

const INSTRUCTIONS: &str = "Edit OpenVideoEditor projects. Start with `docs` topic `agents` for the editing guide. \
Typical loop: app_context (what the person has open) → print_timeline → apply_timeline with text edits and the \
printed rev (one undo step; the receipt gives the new rev and the printed lines of what changed) → \
verify_timeline. Times are seconds. When the app has the project open, edits appear in it at once. \
Skills are playbooks for kinds of edits: `list_skills`, then `read_skill` (also served as prompts and as \
`ove://skills/<id>` resources).";

const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// How long the session keeps a project open after its last call (unify-agent-tool-layer task 4.3): calls in a
/// burst skip opening the bundle and writing its snapshot, and the project is free again for the app and other
/// writers soon after the agent stops. `OVE_MCP_HOLD_MS` overrides it (0 closes after every call).
/// Another writer waits [`ove_tools::bundle::WAIT_FOR_SESSION`] for a held project, which must stay at least this
/// hold plus 5 s (checked by `the_wait_for_a_session_outlasts_its_hold`).
pub const HOLD: std::time::Duration = std::time::Duration::from_secs(2);

fn hold() -> std::time::Duration {
    std::env::var("OVE_MCP_HOLD_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(HOLD)
}

/// Serves MCP on stdin/stdout until stdin closes.
pub fn serve(library: Option<&Path>, project: Option<&str>, turn: Option<&str>) -> std::io::Result<()> {
    let surface = if project.is_some() { Surface::Pinned } else { Surface::Mcp };
    let mut ctx = LinkCtx::open(library, surface, Source::Mcp).map_err(|f| std::io::Error::other(f.message))?;
    if let Some(p) = project {
        // A pinned session: every project tool works on this project, and calls naming another are refused.
        let bundle = ctx.inner.find(p).map_err(|f| std::io::Error::other(f.message))?;
        ctx.inner.binding = ove_tools::Binding { project: Some(bundle), pinned: true };
    }
    // Every revision this session makes records it, so its undo touches only its own changes.
    let session = format!("mcp-{:x}-{:x}", ove_tools::fail::now_ms(), std::process::id());
    ctx.inner.actor =
        Some(Actor { kind: ActorKind::Mcp, session: Some(session), turn: turn.map(str::to_string), client: None });
    let hold = hold();
    ctx.inner.hold = (!hold.is_zero()).then_some(hold);
    // Lines arrive on a reader thread, so the held project can be closed while the client is quiet, and a tool
    // waiting for the client's answer to a confirmation prompt reads from the same stream.
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let end = line.is_err();
            if tx.send(line).is_err() || end {
                break;
            }
        }
    });
    let lines: Lines = Arc::new(Mutex::new(rx));
    // Generation: jobs run in a background `ove generate _run`; without the app, the client's confirmation prompt
    // asks the person to approve a spend.
    ctx.inner.runner = Some(crate::generate::runner(ctx.root().to_path_buf()));
    let prompt_lines = lines.clone();
    ctx.inner.confirm = Some(Box::new(move |card: &Value| elicit(&prompt_lines, card)));
    loop {
        let wait = ctx.inner.idle_in().unwrap_or(std::time::Duration::from_secs(3600));
        let got = lines.lock().unwrap_or_else(PoisonError::into_inner).recv_timeout(wait);
        let line = match got {
            Ok(line) => line?,
            Err(RecvTimeoutError::Timeout) => {
                if let Err(f) = ctx.inner.release_if_idle() {
                    eprintln!("ove mcp: closing the project failed: {}", f.message);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}),
            ),
            Ok(msg) => handle_contained(&msg, &mut ctx),
        };
        if let Some(r) = reply {
            send(&r)?;
        }
    }
    ctx.inner.release().map_err(|f| std::io::Error::other(f.message))
}

/// The client's lines, read on a thread (see [`serve`]).
type Lines = Arc<Mutex<Receiver<std::io::Result<String>>>>;

/// The next line from the client (`None` once it closed the session).
fn next_line(lines: &Lines) -> std::io::Result<Option<String>> {
    match lines.lock().unwrap_or_else(PoisonError::into_inner).recv() {
        Ok(line) => line.map(Some),
        Err(_) => Ok(None),
    }
}

fn send(v: &Value) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{v}")?;
    out.flush()
}

/// Whether the client said in `initialize` that it can show confirmation prompts (MCP elicitation).
static ELICITATION: AtomicBool = AtomicBool::new(false);

/// Asks the client to show the person a priced request (MCP `elicitation/create`) and waits for the answer. Only an
/// explicit accept with `approve: true` approves; declining, cancelling or an error does not. Messages that arrive
/// meanwhile are answered as busy (pings normally).
fn elicit(lines: &Lines, card: &Value) -> Result<bool, Fail> {
    if !ELICITATION.load(Ordering::SeqCst) {
        return Err(ove_tools::ctx::confirm_required());
    }
    let id = format!("ove-confirm-{}", card["quoteId"].as_str().unwrap_or_default());
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "elicitation/create",
        "params": {
            "message": ove_tools::tools::generate::card_text(card),
            "requestedSchema": {
                "type": "object",
                "properties": {"approve": {"type": "boolean", "title": "Approve this spend", "description": "Your provider account is billed; nothing is spent without this."}},
                "required": ["approve"],
            },
        },
    });
    let io = |e: std::io::Error| Fail::new("confirm.required", format!("the confirmation prompt failed: {e}"));
    send(&request).map_err(io)?;
    loop {
        let Some(line) = next_line(lines).map_err(io)? else {
            return Err(Fail::new(
                "confirm.required",
                "the client closed the session before answering; nothing was spent",
            ));
        };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if msg.get("id").and_then(Value::as_str) == Some(id.as_str()) && msg.get("method").is_none() {
            if let Some(e) = msg.get("error") {
                return Err(Fail::new(
                    "confirm.required",
                    format!("the client could not show the confirmation prompt ({e}); nothing was spent"),
                ));
            }
            let r = msg.get("result").cloned().unwrap_or(Value::Null);
            return Ok(r["action"] == "accept" && r["content"]["approve"] == true);
        }
        // A request meanwhile: pings are answered, anything else waits for the person's answer.
        if let (Some(rid), Some(method)) = (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
            let reply = if method == "ping" {
                json!({"jsonrpc": "2.0", "id": rid, "result": {}})
            } else {
                json!({"jsonrpc": "2.0", "id": rid, "error": {"code": -32603, "message": "busy: waiting for the person to answer a price prompt"}})
            };
            send(&reply).map_err(io)?;
        }
    }
}

pub fn handle(msg: &Value, ctx: &mut dyn ToolCtx) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    // Notifications (no id) get no reply.
    let id = id?;
    let surface = ctx.surface();
    let result = match method {
        "initialize" => {
            if let Some(name) = params.pointer("/clientInfo/name").and_then(Value::as_str) {
                ctx.set_client(name);
            }
            // A client that can show confirmation prompts lets a person approve a spend without the app.
            ELICITATION
                .store(params.pointer("/capabilities/elicitation").is_some_and(Value::is_object), Ordering::SeqCst);
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOLS[0]);
            let version = if PROTOCOLS.contains(&asked) { asked } else { PROTOCOLS[0] };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {
                    "tools": {"listChanged": false},
                    "prompts": {"listChanged": false},
                    "resources": {"listChanged": false},
                },
                "serverInfo": {"name": "openvideoeditor", "title": "OpenVideoEditor", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(ove_tools::mcp_list(surface)),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            match ove_tools::find(name).filter(|t| t.on(surface)) {
                None => {
                    let f = ove_tools::unknown(name, surface);
                    Err((
                        -32602,
                        format!("unknown tool: {name} ({})", f.message),
                        Some(json!({"code": f.code, "details": f.details})),
                    ))
                }
                Some(_) => Ok(call(ctx, name, &args)),
            }
        }
        // Skills (add-agent-skills-and-connectors D3): each valid skill is a prompt and an `ove://skills/<id>` resource.
        "prompts/list" => skills(ctx).map(|s| ove_tools::skills::mcp_prompts(&s)),
        "prompts/get" => skills(ctx).and_then(|s| {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            ove_tools::skills::mcp_prompt_get(&s, name).map_err(skill_error)
        }),
        "resources/list" => skills(ctx).map(|s| ove_tools::skills::mcp_resources(&s)),
        "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
        "resources/read" => skills(ctx).and_then(|s| {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            ove_tools::skills::mcp_resource_read(&s, uri).map_err(skill_error)
        }),
        other => Err((-32601, format!("method not found: {other}"), None)),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message, data)) => {
            let mut e = json!({"code": code, "message": message});
            if let Some(d) = data {
                e["data"] = d;
            }
            json!({"jsonrpc": "2.0", "id": id, "error": e})
        }
    })
}

type RpcError = (i64, String, Option<Value>);

fn skill_error(f: ove_tools::Fail) -> RpcError {
    (-32602, f.message.clone(), Some(json!({"code": f.code, "details": f.details})))
}

fn skills(ctx: &mut dyn ToolCtx) -> Result<ove_tools::skills::SkillStore, RpcError> {
    ctx.skills().map_err(|f| (-32603, f.message.clone(), Some(json!({"code": f.code}))))
}

/// [`handle`], with a panic anywhere in the message's handling answered as a JSON-RPC internal error, so one bad
/// call does not end the session.
pub fn handle_contained(msg: &Value, ctx: &mut dyn ToolCtx) -> Option<Value> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(msg, ctx))) {
        Ok(reply) => reply,
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            let id = msg.get("id").cloned()?;
            Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("internal error: {what}")}}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ove_store::Library;
    use ove_store::project::Project;
    use ove_tools::Editor;

    use super::*;

    /// A context whose handlers panic on reading (stands in for a tool that blows up).
    struct Panicky(Library);

    impl ToolCtx for Panicky {
        fn surface(&self) -> Surface {
            Surface::Mcp
        }
        fn source(&self) -> Source {
            Source::Mcp
        }
        fn bundle(&mut self, _: Option<&str>) -> Result<PathBuf, Fail> {
            panic!("tool blew up")
        }
        fn read(&mut self, _: &Path, _: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
            panic!("tool blew up")
        }
        fn edit(&mut self, _: &Path, _: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>) -> Result<(), Fail> {
            panic!("tool blew up")
        }
        fn app(&mut self, _: &str, _: Value) -> Result<Value, Fail> {
            panic!("tool blew up")
        }
        fn library(&mut self) -> Result<&Library, Fail> {
            Ok(&self.0)
        }
    }

    fn call_msg(name: &str, arguments: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
    }

    #[test]
    fn a_panicking_tool_is_an_error_reply_not_the_end() {
        let d = tempfile::tempdir().unwrap();
        let mut ctx = Panicky(Library::open(d.path(), crate::common::clock()).unwrap());
        let reply = handle_contained(&call_msg("show_project", json!({"project": "p"})), &mut ctx).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["code"], -32603);
        assert!(reply["error"]["message"].as_str().unwrap().contains("tool blew up"));
        // The next message is served normally.
        let ok = handle_contained(&call_msg("list_projects", json!({})), &mut ctx).unwrap();
        assert_eq!(ok["result"]["isError"], false, "{ok}");
        // A notification that panics gets no reply.
        let note = json!({"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "show_project", "arguments": {"project": "p"}}});
        assert!(handle_contained(&note, &mut ctx).is_none());
    }

    /// A second writer waits for a session's held project long enough for the hold to end, with 5 s of margin
    /// for a loaded machine; else a second agent gets `project.locked` from a session that is about to let go.
    #[test]
    fn the_wait_for_a_session_outlasts_its_hold() {
        let margin = std::time::Duration::from_secs(5);
        assert!(
            ove_tools::bundle::WAIT_FOR_SESSION >= HOLD + margin,
            "WAIT_FOR_SESSION {:?} must be at least HOLD {HOLD:?} + {margin:?}",
            ove_tools::bundle::WAIT_FOR_SESSION
        );
    }

    #[test]
    fn unknown_tools_are_protocol_errors_naming_the_closest_tools() {
        let d = tempfile::tempdir().unwrap();
        let mut ctx = Panicky(Library::open(d.path(), crate::common::clock()).unwrap());
        let reply = handle(&call_msg("print_timline", json!({})), &mut ctx).unwrap();
        assert_eq!(reply["error"]["code"], -32602);
        assert_eq!(reply["error"]["data"]["code"], "tool.unknown");
        assert_eq!(reply["error"]["data"]["details"]["suggestions"][0], "print_timeline", "{reply}");
        // In-app-only tools are not served.
        let reply = handle(&call_msg("ask_user", json!({"question": "?"})), &mut ctx).unwrap();
        assert_eq!(reply["error"]["data"]["code"], "tool.unknown");
    }
}
