//! A local stand-in for the Messages API, for tests and for demonstrating the assistant without an API key
//! (`OVE_ANTHROPIC_BASE_URL=http://127.0.0.1:<port>`, plus `OVE_DEV_ENDPOINTS=1` in a release build). It serves
//! scripted Server-Sent Event responses.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

/// A scripted reply.
pub enum Reply {
    /// An SSE body sent in chunks with a delay between them.
    Stream { chunks: Vec<String>, delay: Duration },
    /// An HTTP error status with a JSON body.
    Status(u16, String),
    /// A 200 response with a JSON body (non-streaming endpoints such as token counting).
    Json(String),
}

type Handler = dyn Fn(usize, &str, &Value) -> Reply + Send + Sync;

/// A running mock server. Requests it received are kept for inspection.
pub struct MockServer {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Value>>>,
}

impl MockServer {
    pub fn start(handler: impl Fn(usize, &Value) -> Reply + Send + Sync + 'static) -> MockServer {
        Self::start_on("127.0.0.1:0", handler)
    }

    pub fn start_on(addr: &str, handler: impl Fn(usize, &Value) -> Reply + Send + Sync + 'static) -> MockServer {
        Self::start_routed_on(addr, move |n, _path, req| handler(n, req))
    }

    /// Like [`MockServer::start`], with the request path (`/v1/messages`, `/v1/messages/count_tokens`, …) passed
    /// to the handler.
    pub fn start_routed(handler: impl Fn(usize, &str, &Value) -> Reply + Send + Sync + 'static) -> MockServer {
        Self::start_routed_on("127.0.0.1:0", handler)
    }

    #[allow(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "test support: tests and the demo example cannot run without the listening socket"
    )]
    pub fn start_routed_on(
        addr: &str,
        handler: impl Fn(usize, &str, &Value) -> Reply + Send + Sync + 'static,
    ) -> MockServer {
        let listener = TcpListener::bind(addr).expect("bind mock server");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let reqs = requests.clone();
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (reqs, handler) = (reqs.clone(), handler.clone());
                std::thread::spawn(move || {
                    let _ = serve(stream, &reqs, handler.as_ref());
                });
            }
        });
        MockServer { url, requests }
    }
}

fn serve(mut stream: TcpStream, reqs: &Mutex<Vec<Value>>, handler: &Handler) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut len = 0usize;
    let mut path = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let l = line.trim_end();
        if path.is_empty() {
            // Request line: `POST /v1/messages HTTP/1.1`.
            path = l.split_whitespace().nth(1).unwrap_or("/").to_string();
            continue;
        }
        if l.is_empty() {
            break;
        }
        if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let n = {
        let mut r = reqs.lock().unwrap_or_else(PoisonError::into_inner);
        r.push(req.clone());
        r.len() - 1
    };
    match handler(n, &path, &req) {
        Reply::Status(code, body) => {
            write!(
                stream,
                "HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
        }
        Reply::Json(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
        }
        Reply::Stream { chunks, delay } => {
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")?;
            for c in chunks {
                stream.write_all(c.as_bytes())?;
                stream.flush()?;
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }
        }
    }
    Ok(())
}

fn event(v: Value) -> String {
    format!("event: {}\ndata: {}\n\n", v["type"].as_str().unwrap_or("x"), v)
}

/// SSE chunks for a reply made of text (split into words) and optional tool calls.
pub fn sse_reply(text: &str, tool_calls: &[(&str, &str, Value)]) -> Vec<String> {
    let mut out = vec![event(
        json!({"type": "message_start", "message": {"id": "msg_mock", "model": "mock", "usage": {"input_tokens": 1200, "cache_read_input_tokens": 3000}}}),
    )];
    let mut i = 0;
    if !text.is_empty() {
        out.push(event(
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        ));
        for w in text.split_inclusive(' ') {
            out.push(event(
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": w}}),
            ));
        }
        out.push(event(json!({"type": "content_block_stop", "index": 0})));
        i = 1;
    }
    for (id, name, input) in tool_calls {
        out.push(event(json!({"type": "content_block_start", "index": i, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}})));
        let s = input.to_string();
        let (a, b) = s.split_at(s.len() / 2);
        for part in [a, b] {
            out.push(event(json!({"type": "content_block_delta", "index": i, "delta": {"type": "input_json_delta", "partial_json": part}})));
        }
        out.push(event(json!({"type": "content_block_stop", "index": i})));
        i += 1;
    }
    let stop = if tool_calls.is_empty() { "end_turn" } else { "tool_use" };
    out.push(event(json!({"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": 180}})));
    out.push(event(json!({"type": "message_stop"})));
    out
}

/// The text of the most recent tool result in a request, with its tool_use id.
fn last_tool_result(req: &Value) -> Option<(String, String)> {
    let last = req["messages"].as_array()?.last()?;
    let blocks = last["content"].as_array()?;
    let b = blocks.iter().rev().find(|b| b["type"] == "tool_result")?;
    Some((b["tool_use_id"].as_str()?.to_string(), b["content"].as_str().unwrap_or_default().to_string()))
}

/// Answers a token-counting request (the key check) with a fixed count; `None` for other paths.
pub fn count_tokens_reply(path: &str) -> Option<Reply> {
    path.ends_with("/count_tokens").then(|| Reply::Json(json!({"input_tokens": 8}).to_string()))
}

/// A deterministic "assistant" for demos and tests: reads the timeline, adds a title on the first track, verifies,
/// and reports.
pub fn demo_brain(_n: usize, req: &Value) -> Reply {
    let delay = Duration::from_millis(25);
    let chunks = match last_tool_result(req) {
        None => sse_reply("I'll take a look at the timeline first.", &[("tu_print", "print_timeline", json!({}))]),
        Some((id, text)) if id == "tu_print" => {
            let rev = text.lines().find_map(|l| {
                l.trim().trim_start_matches('*').trim().strip_prefix("rev:").map(|r| r.trim().to_string())
            });
            let anchor = text.lines().find(|l| l.trim_start().starts_with("<track ")).map(|l| format!("{l}\n"));
            match (rev, anchor) {
                (Some(rev), Some(anchor)) => {
                    let new = format!(
                        "{anchor}      <text name=\"Opening title\" start={{0}} end={{3}} textContent=\"Welcome\" fontSize={{96}} color=\"#ffffff\" />\n"
                    );
                    sse_reply(
                        "Adding an opening title for the first three seconds.",
                        &[(
                            "tu_apply",
                            "apply_timeline",
                            json!({"rev": rev.parse::<i64>().unwrap_or(0), "edits": [{"oldText": anchor, "newText": new}]}),
                        )],
                    )
                }
                _ => sse_reply("The timeline has no tracks yet, so there is nowhere to put a title.", &[]),
            }
        }
        Some((id, _)) if id == "tu_apply" => sse_reply("", &[("tu_verify", "verify_timeline", json!({}))]),
        Some(_) => sse_reply(
            "Done: I added a 3-second \"Welcome\" title at the start of the top track. Undo removes it in one step.",
            &[],
        ),
    };
    Reply::Stream { chunks, delay }
}
