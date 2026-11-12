//! Turn loop against a local mock of the Messages API: streaming, tool use on a real project, errors and retries,
//! cancellation, and resuming a conversation from its log.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ove_agent::chat::{SUMMARY_PREFIX, estimate_tokens, validate_messages};
use ove_agent::mock::{MockServer, Reply, count_tokens_reply, demo_brain, sse_reply};
use ove_agent::tools::{self, ApplyRequest, ToolHost};
use ove_agent::{AgentEvent, ChatStore, Client, ProjectCap, Record, TurnConfig, run_turn};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use serde_json::json;

/// A tool host over a real project bundle.
struct TestHost {
    project: Project,
    answers: Vec<String>,
    /// The revision last reported by `take_revisions`.
    seen: i64,
}

impl TestHost {
    fn new(project: Project, answers: Vec<String>) -> Self {
        let seen = project.rev();
        TestHost { project, answers, seen }
    }
}

impl ToolHost for TestHost {
    fn print_timeline(&mut self, from: Option<usize>, lines: Option<usize>) -> Result<String, String> {
        Ok(tools::print(&self.project, from, lines))
    }
    fn apply_timeline(&mut self, req: ApplyRequest) -> Result<String, String> {
        tools::apply(&mut self.project, req)
    }
    fn verify_timeline(&mut self) -> Result<String, String> {
        Ok(tools::verify(&self.project))
    }
    fn list_media(&mut self) -> Result<String, String> {
        Ok(tools::list_media(&self.project))
    }
    fn undo_last(&mut self) -> Result<String, String> {
        tools::undo_last(&mut self.project)
    }
    fn ask_user(&mut self, _id: &str, _q: &str, _o: &[String]) -> Result<String, String> {
        self.answers.pop().ok_or_else(|| "no answer".into())
    }
    fn take_revisions(&mut self) -> Vec<i64> {
        let rev = self.project.rev();
        if rev == self.seen {
            return vec![];
        }
        self.seen = rev;
        vec![rev]
    }
}

fn project(dir: &std::path::Path) -> Project {
    let lib = Library::open(dir, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = lib.create("Demo").unwrap();
    p.apply(
        vec![
            Op::AddTrack { track: json!({"id": "t_text", "name": "Text", "order": 0, "type": "video"}) },
            Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 1, "type": "video"}) },
        ],
        Source::Gui,
        None,
    )
    .unwrap();
    p
}

fn fast_client(url: &str) -> Client {
    let mut c = Client::with_base_url("sk-test-key-0123456789abcdef", url);
    c.backoff = vec![Duration::from_millis(10); 4];
    c
}

#[test]
fn streams_text_and_logs_the_turn() {
    let server = MockServer::start(|_, _| Reply::Stream {
        chunks: sse_reply("Hello there, editor.", &[]),
        delay: Duration::ZERO,
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("hi", "claude-sonnet-5-5").unwrap();
    let mut events = vec![];
    let out = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "hi",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    assert_eq!(out.text, "Hello there, editor.");
    let deltas: String = events
        .iter()
        .filter_map(|e| if let AgentEvent::TextDelta { text } = e { Some(text.clone()) } else { None })
        .collect();
    assert_eq!(deltas, "Hello there, editor.");
    assert!(matches!(events.last(), Some(AgentEvent::TurnDone { stopped: false, .. })));
    let recs = store.records(&chat.id);
    assert!(matches!(recs.last(), Some(Record::Result { is_error: false, .. })));
    // The request has the cacheable prefix and current API shape.
    let req = &server.requests.lock().unwrap()[0];
    assert_eq!(req["model"], "claude-sonnet-5-5");
    assert_eq!(req["stream"], true);
    assert_eq!(req["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(req["thinking"]["type"], "adaptive");
    assert_eq!(req["fallbacks"], "default");
    assert!(req["tools"].as_array().unwrap().iter().all(|t| t["eager_input_streaming"] == true));
    assert!(req.get("temperature").is_none());
}

#[test]
fn tool_loop_edits_the_project_as_one_agent_revision() {
    let server = MockServer::start(demo_brain);
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("add a title", "claude-sonnet-5-5").unwrap();
    let before = host.project.rev();
    let mut events = vec![];
    let out = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "add a title",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    assert!(out.text.contains("Welcome"));
    // print → apply → verify → answer.
    assert_eq!(server.requests.lock().unwrap().len(), 4);
    let calls: Vec<String> = events
        .iter()
        .filter_map(|e| if let AgentEvent::ToolCall { name, .. } = e { Some(name.clone()) } else { None })
        .collect();
    assert_eq!(calls, ["print_timeline", "apply_timeline", "verify_timeline"]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolResult { ok: true, summary, .. } if summary.contains("added 1 item")))
    );
    let doc = host.project.doc();
    let title = doc.project.items.values().find(|i| i.kind == "text").expect("title added");
    assert_eq!(title.fields["textContent"], "Welcome");
    assert_eq!(title.track_id, "t_text");
    let latest = host.project.latest_record().unwrap().unwrap();
    assert_eq!(latest.source, Source::Agent);
    assert!(host.project.rev() > before);
    // The second request replays the first reply's blocks unchanged and answers its tool call.
    let reqs = server.requests.lock().unwrap();
    let m = reqs[1]["messages"].as_array().unwrap();
    assert_eq!(m[1]["role"], "assistant");
    assert_eq!(m[2]["content"][0]["type"], "tool_result");
    assert_eq!(m[2]["content"][0]["tool_use_id"], "tu_print");
    drop(reqs);
    // undo_last reverts it.
    let msg = host.undo_last().unwrap();
    assert!(msg.starts_with("Undid"));
    assert!(host.project.doc().project.items.values().all(|i| i.kind != "text"));
    assert!(host.undo_last().is_err(), "the next latest change is the user's");
}

#[test]
fn stale_rev_and_bad_edits_are_reported_to_the_model() {
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let e = host
        .apply_timeline(ApplyRequest { rev: Some(1), edits: Some(vec![]), source: None, prefer: None })
        .unwrap_err();
    assert!(e.contains("Print it again"), "{e}");
    let rev = host.project.rev();
    let bad =
        ove_projection::textedit::TextEdit { old_text: "kind=\"video\"".into(), new_text: "kind=\"video\"".into() };
    let e = host
        .apply_timeline(ApplyRequest { rev: Some(rev), edits: Some(vec![bad]), source: None, prefer: None })
        .unwrap_err();
    assert!(e.contains("edit.ambiguous"), "{e}");
    let text = host.print_timeline(None, None).unwrap();
    let broken = text.replace("name=\"Main\"", "name={1 + 1}");
    let e =
        host.apply_timeline(ApplyRequest { rev: None, edits: None, source: Some(broken), prefer: None }).unwrap_err();
    assert!(e.contains("literal.only"), "{e}");
    // Invalid inputs never reach the host.
    assert!(tools::dispatch(&mut host, "x", "apply_timeline", &json!({"edits": [], "source": "x"})).is_err());
    assert!(tools::dispatch(&mut host, "x", "print_timeline", &json!({"from": 0})).is_err());
    assert!(tools::dispatch(&mut host, "x", "nope", &json!({})).is_err());
}

#[test]
fn check_code_reports_problems_with_positions() {
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let ok = tools::dispatch(&mut host, "x", "check_code", &json!({"composition": {"template": "counter"}})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&ok).unwrap();
    assert_eq!(v["ok"], true, "{ok}");
    assert_eq!(v["diagnostics"], json!([]));
    let src = "export default function C({ time }) {\n  return fetch('x');\n}";
    let bad = tools::check_code(Some(&host.project), &json!({"source": src}), 2.0);
    assert!(bad.contains("error [global] 2:10 `fetch` is not available"), "{bad}");
    assert!(tools::dispatch(&mut host, "x", "check_code", &json!({"composition": "nope"})).is_err());
    assert!(tools::dispatch(&mut host, "x", "check_code", &json!({"composition": {}, "duration": -1})).is_err());
    assert!(tools::definitions().iter().any(|t| t["name"] == "check_code"));
}

#[test]
fn errors_map_and_retries_recover() {
    let server = MockServer::start(|_, _| {
        Reply::Status(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into(),
        )
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("hi", "m").unwrap();
    let mut events = vec![];
    let e = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "hi",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap_err();
    assert_eq!(e.code, "auth.invalidKey");
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { code, .. } if code == "auth.invalidKey")));

    // Two overloads, then success.
    let flaky = MockServer::start(|n, _| {
        if n < 2 {
            Reply::Status(529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#.into())
        } else {
            Reply::Stream { chunks: sse_reply("ok", &[]), delay: Duration::ZERO }
        }
    });
    let mut events = vec![];
    let out = run_turn(
        &fast_client(&flaky.url),
        &store,
        &chat.id,
        "again",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    assert_eq!(out.text, "ok");
    assert_eq!(events.iter().filter(|e| matches!(e, AgentEvent::Retrying { .. })).count(), 2);

    // No server at all.
    let e = run_turn(
        &fast_client("http://127.0.0.1:9"),
        &store,
        &chat.id,
        "x",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(e.code, "net.unreachable");
}

#[test]
fn cost_uses_the_served_model_and_counts_streams_that_fail_part_way() {
    let ev = |v: serde_json::Value| format!("event: {}\ndata: {}\n\n", v["type"].as_str().unwrap(), v);
    let server = MockServer::start(move |n, _| {
        let chunks = if n == 0 {
            // Opus starts answering, then the stream breaks off with an overload (retried: nothing was shown).
            vec![
                ev(
                    json!({"type": "message_start", "message": {"id": "m0", "model": "claude-opus-5-5", "usage": {"input_tokens": 1_000_000}}}),
                ),
                ev(json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})),
            ]
        } else {
            // A server-side fallback answers with a dated Haiku id.
            sse_reply("done", &[])
                .into_iter()
                .map(|c| c.replace("\"model\":\"mock\"", "\"model\":\"claude-haiku-5-5-20261001\""))
                .collect()
        };
        Reply::Stream { chunks, delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("hi", "claude-sonnet-5-5").unwrap();
    let mut events = vec![];
    run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "hi",
        &mut host,
        &TurnConfig::default(), // asks for Sonnet
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    let Some(AgentEvent::TurnDone { usage, cost_usd, .. }) = events.last() else { panic!("{events:?}") };
    // The failed attempt's input is billed at Opus prices, the answer at Haiku prices (sse_reply: 1200 input,
    // 3000 cache read, 180 output).
    assert_eq!(usage.input_tokens, 1_000_000 + 1200);
    let opus = 4.0;
    let haiku = (1200.0 * 0.10 + 3000.0 * 0.01 + 180.0 * 0.50) / 1e6;
    assert!((cost_usd - (opus + haiku)).abs() < 1e-9, "{cost_usd}");
    let recs = store.records(&chat.id);
    assert!(matches!(recs.last(), Some(Record::Result { cost_usd: c, .. }) if (c - cost_usd).abs() < 1e-12));
}

#[test]
fn stop_cancels_mid_stream_and_keeps_history_valid() {
    let server = MockServer::start(|n, _| {
        if n == 0 {
            let mut chunks = sse_reply(&"word ".repeat(200), &[]);
            chunks.truncate(150);
            Reply::Stream { chunks, delay: Duration::from_millis(20) }
        } else {
            Reply::Stream { chunks: sse_reply("resumed", &[]), delay: Duration::ZERO }
        }
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("long", "m").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c2.store(true, Ordering::SeqCst);
    });
    let started = std::time::Instant::now();
    let out = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "write a lot",
        &mut host,
        &TurnConfig::default(),
        &cancel,
        &mut |_| {},
    )
    .unwrap();
    assert!(out.stopped);
    assert!(started.elapsed() < Duration::from_secs(2));
    // Resume: the next turn sends the earlier user message too.
    let out = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "go on",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(out.text, "resumed");
    let reqs = server.requests.lock().unwrap();
    let m = reqs.last().unwrap()["messages"].as_array().unwrap().clone();
    assert_eq!(m.len(), 1, "two user turns with no assistant reply merge into one user message");
    assert_eq!(m[0]["content"].as_array().unwrap().len(), 2);
}

#[test]
fn ask_user_round_trip() {
    let server = MockServer::start(|n, _| {
        let chunks = if n == 0 {
            sse_reply(
                "",
                &[("tu_q", "ask_user", json!({"question": "Which aspect ratio?", "options": ["16:9", "9:16"]}))],
            )
        } else {
            sse_reply("Going vertical.", &[])
        };
        Reply::Stream { chunks, delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec!["9:16".into()]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("q", "m").unwrap();
    let mut events = vec![];
    run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "make it for phones",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Question { options, .. } if options.len() == 2)));
    assert!(store.records(&chat.id).iter().any(|r| matches!(r, Record::Question { .. })));
    let reqs = server.requests.lock().unwrap();
    assert_eq!(reqs[1]["messages"][2]["content"][0]["content"], "9:16");
}

/// A server whose model never stops calling a (harmless) tool; each reply costs about $0.0045 at Sonnet prices
/// (sse_reply: 1200 input, 3000 cache read, 180 output tokens).
fn endless_tool_calls() -> MockServer {
    MockServer::start(|n, _| Reply::Stream {
        chunks: sse_reply("Checking again.", &[(&format!("tu_{n}"), "verify_timeline", json!({}))]),
        delay: Duration::ZERO,
    })
}

fn turn_with(server: &MockServer, cfg: &TurnConfig) -> (ove_agent::TurnSummary, Vec<Record>, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("loop", "claude-sonnet-5-5").unwrap();
    let out = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "keep checking",
        &mut host,
        cfg,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let recs = store.records(&chat.id);
    (out, recs, d)
}

#[test]
fn spend_cap_per_turn_stops_between_requests_and_says_how_to_raise_it() {
    let server = endless_tool_calls();
    let cfg = TurnConfig { max_cost_usd: Some(0.01), ..TurnConfig::default() };
    let (out, recs, _d) = turn_with(&server, &cfg);
    // $0.0045 per request: after three the turn has spent $0.0135 ≥ $0.01, so no fourth request is sent.
    assert_eq!(server.requests.lock().unwrap().len(), 3);
    let note = out.note.unwrap();
    assert!(note.contains("spend cap of $0.01"), "{note}");
    assert!(note.contains("Settings → Assistant"), "{note}");
    assert!((out.cost_usd - 0.0135).abs() < 1e-9, "{}", out.cost_usd);
    // The note and the cost survive in the log.
    let Some(Record::Result { note: Some(n), cost_usd, time, .. }) = recs.last() else { panic!("{recs:?}") };
    assert_eq!(*n, note);
    assert!((cost_usd - 0.0135).abs() < 1e-9);
    assert!(*time > 0);
}

#[test]
fn request_and_time_limits_name_the_limit() {
    let server = endless_tool_calls();
    let cfg = TurnConfig { max_requests: 2, max_cost_usd: None, ..TurnConfig::default() };
    let (out, _, _d) = turn_with(&server, &cfg);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    assert!(out.note.unwrap().contains("limit of 2 requests"));

    // Each reply streams for well over 50 ms.
    let server = MockServer::start(|n, _| Reply::Stream {
        chunks: sse_reply("Checking again.", &[(&format!("tu_{n}"), "verify_timeline", json!({}))]),
        delay: Duration::from_millis(15),
    });
    let cfg = TurnConfig { max_duration: Duration::from_millis(50), ..TurnConfig::default() };
    let (out, _, _d) = turn_with(&server, &cfg);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert!(out.note.unwrap().contains("Max minutes per turn"));
}

#[test]
fn project_cap_blocks_a_turn_before_it_starts_and_stops_one_that_reaches_it() {
    let server = endless_tool_calls();
    let reached = TurnConfig { project_cap: Some(ProjectCap { cap_usd: 5.0, spent_usd: 5.0 }), ..Default::default() };
    let (out, _, _d) = turn_with(&server, &reached);
    assert!(server.requests.lock().unwrap().is_empty(), "nothing is sent once the cap is reached");
    let note = out.note.unwrap();
    assert!(note.contains("monthly assistant spend cap of $5.00"), "{note}");
    assert!(note.contains("Spend cap per project per month"), "{note}");

    let server = endless_tool_calls();
    let close = TurnConfig { project_cap: Some(ProjectCap { cap_usd: 5.0, spent_usd: 4.995 }), ..Default::default() };
    let (out, _, _d) = turn_with(&server, &close);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    assert!(out.note.unwrap().contains("monthly"));
}

#[test]
fn a_turn_records_the_revisions_it_made() {
    let server = MockServer::start(demo_brain);
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("add a title", "claude-sonnet-5-5").unwrap();
    let out = run_turn(
        &fast_client(&server.url),
        &store,
        &chat.id,
        "add a title",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(out.revs, vec![host.project.rev()]);
    let recs = store.records(&chat.id);
    let user_id = recs.iter().find_map(|r| if let Record::User { id, .. } = r { Some(id.clone()) } else { None });
    assert_eq!(user_id.as_deref(), Some(out.turn_id.as_str()));
    let logged: Vec<(String, Vec<i64>)> = recs
        .iter()
        .filter_map(
            |r| if let Record::Revisions { turn, revs } = r { Some((turn.clone(), revs.clone())) } else { None },
        )
        .collect();
    assert_eq!(logged, vec![(out.turn_id.clone(), out.revs.clone())]);
}

#[test]
fn key_check_uses_token_counting_and_reports_the_provider_error() {
    let ok = MockServer::start_routed(|_, path, _| count_tokens_reply(path).unwrap_or(Reply::Status(404, "{}".into())));
    fast_client(&ok.url).check_key("claude-sonnet-5-5").unwrap();
    let req = &ok.requests.lock().unwrap()[0];
    assert_eq!(req["model"], "claude-sonnet-5-5");
    assert!(req.get("max_tokens").is_none(), "nothing is generated");

    let bad = MockServer::start_routed(|_, _, _| {
        Reply::Status(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into(),
        )
    });
    let e = fast_client(&bad.url).check_key("claude-sonnet-5-5").unwrap_err();
    assert_eq!(e.code, "auth.invalidKey");
    assert!(e.message.contains("rejected"));
    let e = fast_client("http://127.0.0.1:9").check_key("claude-sonnet-5-5").unwrap_err();
    assert_eq!(e.code, "net.unreachable");
}

#[test]
fn a_200_turn_chat_is_summarised_to_stay_under_the_limit_with_tool_pairs_intact() {
    let server = MockServer::start(|_, req| {
        // The summary call has no tools; the turn itself does.
        let text = if req.get("tools").is_none() { "The user built a 200-step edit." } else { "ok" };
        Reply::Stream { chunks: sse_reply(text, &[]), delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("long", "claude-sonnet-5-5").unwrap();
    let big = "x".repeat(2_000);
    for i in 0..200 {
        let recs = [
            Record::User { id: format!("u_{i}"), text: format!("step {i}"), time: i },
            Record::Assistant {
                message_id: format!("m{i}a"),
                model: "claude-sonnet-5-5".into(),
                blocks: vec![
                    json!({"type": "thinking", "thinking": "", "signature": "sig"}),
                    json!({"type": "tool_use", "id": format!("t{i}a"), "name": "print_timeline", "input": {}}),
                    json!({"type": "tool_use", "id": format!("t{i}b"), "name": "list_media", "input": {}}),
                ],
            },
            Record::ToolResult { tool_use_id: format!("t{i}a"), text: big.clone(), is_error: false },
            Record::ToolResult { tool_use_id: format!("t{i}b"), text: "[]".into(), is_error: false },
            Record::Assistant {
                message_id: format!("m{i}b"),
                model: "claude-sonnet-5-5".into(),
                blocks: vec![json!({"type": "text", "text": format!("Did step {i}.")})],
            },
        ];
        for r in &recs {
            store.append(&chat.id, r).unwrap();
        }
    }
    let limit = 20_000;
    let cfg = TurnConfig { compact_at_tokens: Some(limit), ..TurnConfig::default() };
    let before = ove_agent::agent::request_body(&cfg, &ove_agent::chat::messages(&store.records(&chat.id)));
    assert!(estimate_tokens(&before) > 5 * limit, "the synthetic chat is well over the limit");
    for turn in ["and one more", "and another"] {
        run_turn(
            &fast_client(&server.url),
            &store,
            &chat.id,
            turn,
            &mut host,
            &cfg,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
    }
    let reqs = server.requests.lock().unwrap();
    let summaries = reqs.iter().filter(|r| r.get("tools").is_none()).count();
    assert_eq!(summaries, 1, "one cheap call; the next turn still fits");
    let summary_req = reqs.iter().find(|r| r.get("tools").is_none()).unwrap();
    assert_eq!(summary_req["model"], "claude-haiku-5-5");
    assert!(summary_req["max_tokens"].as_u64().unwrap() <= 2000);
    for main in reqs.iter().filter(|r| r.get("tools").is_some()) {
        assert!(estimate_tokens(main) < limit, "{} tokens", estimate_tokens(main));
        let msgs = main["messages"].as_array().unwrap();
        validate_messages(msgs).unwrap();
        assert!(msgs[0]["content"][0]["text"].as_str().unwrap().starts_with(SUMMARY_PREFIX));
        assert!(msgs[0]["content"][0]["text"].as_str().unwrap().contains("200-step edit"));
    }
    // The log keeps everything for the chat view, plus the summary.
    let recs = store.records(&chat.id);
    assert_eq!(recs.iter().filter(|r| matches!(r, Record::User { .. })).count(), 202);
    assert_eq!(recs.iter().filter(|r| matches!(r, Record::Summary { .. })).count(), 1);
}

#[test]
#[ignore = "live API call; needs ANTHROPIC_API_KEY"]
fn live_turn() {
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else { return };
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost::new(project(d.path()), vec![]);
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("live", "claude-sonnet-5-5").unwrap();
    let out = run_turn(
        &Client::new(key),
        &store,
        &chat.id,
        "Add a 3 second title saying Hello on the Text track.",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert!(!out.text.is_empty());
    assert!(host.project.doc().project.items.values().any(|i| i.kind == "text"));
}
