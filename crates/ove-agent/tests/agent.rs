//! Turn loop against a local mock of the Messages API: streaming, tool use on a real project, errors and retries,
//! cancellation, and resuming a conversation from its log.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ove_agent::mock::{MockServer, Reply, demo_brain, sse_reply};
use ove_agent::tools::{self, ApplyRequest, ToolHost};
use ove_agent::{AgentEvent, ChatStore, Client, Record, TurnConfig, run_turn};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use serde_json::json;

/// A tool host over a real project bundle.
struct TestHost {
    project: Project,
    answers: Vec<String>,
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
    let mut host = TestHost { project: project(d.path()), answers: vec![] };
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
    let mut host = TestHost { project: project(d.path()), answers: vec![] };
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
    let mut host = TestHost { project: project(d.path()), answers: vec![] };
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
fn errors_map_and_retries_recover() {
    let server = MockServer::start(|_, _| {
        Reply::Status(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into(),
        )
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost { project: project(d.path()), answers: vec![] };
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
    let mut host = TestHost { project: project(d.path()), answers: vec![] };
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
    let mut host = TestHost { project: project(d.path()), answers: vec!["9:16".into()] };
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

#[test]
#[ignore = "live API call; needs ANTHROPIC_API_KEY"]
fn live_turn() {
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else { return };
    let d = tempfile::tempdir().unwrap();
    let mut host = TestHost { project: project(d.path()), answers: vec![] };
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
