//! Mentions in a turn (change add-agent-chat-context-and-activity, task 1.3): a mocked model edits a mentioned item
//! straight from the excerpt it was sent — no print call — and the edit survives a concurrent change in the editor.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ove_agent::mention::{MentionRef, resolve};
use ove_agent::mock::{MockServer, Reply, sse_reply};
use ove_agent::tools::{self, ApplyRequest, ToolHost};
use ove_agent::{AgentEvent, ChatStore, Client, Record, TurnConfig, run_turn_with};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use serde_json::{Map, Value, json};

/// What the person does in the editor while the assistant works (before its apply lands).
#[derive(Clone, Copy, PartialEq)]
enum Meanwhile {
    Nothing,
    MoveTitle,
    DeleteTitle,
}

struct Host {
    project: Project,
    meanwhile: Meanwhile,
}

impl ToolHost for Host {
    fn print_timeline(&mut self, from: Option<usize>, lines: Option<usize>) -> Result<String, String> {
        Ok(tools::print(&self.project, from, lines))
    }
    fn apply_timeline(&mut self, req: ApplyRequest) -> Result<String, String> {
        match self.meanwhile {
            Meanwhile::Nothing => {}
            Meanwhile::MoveTitle => {
                let mut set = Map::new();
                set.insert("startTime".into(), json!(5));
                set.insert("endTime".into(), json!(8));
                self.project.apply(vec![Op::UpdateItem { id: "i_title".into(), set }], Source::Gui, None).unwrap();
            }
            Meanwhile::DeleteTitle => {
                self.project.apply(vec![Op::RemoveItems { ids: vec!["i_title".into()] }], Source::Gui, None).unwrap();
            }
        }
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
        Err("no".into())
    }
}

fn project(dir: &std::path::Path) -> Project {
    let lib = Library::open(dir, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = lib.create("Mentions").unwrap();
    p.apply(
        vec![
            Op::AddTrack { track: json!({"id": "t_text", "name": "Text", "order": 0, "type": "video"}) },
            Op::AddItem {
                item: json!({"id": "i_title", "trackId": "t_text", "type": "text", "startTime": 0, "endTime": 3, "name": "Title", "textContent": "Hello", "fontSize": 96}),
            },
        ],
        Source::Gui,
        None,
    )
    .unwrap();
    p
}

/// The model's side: it reads the mention block of the first message and edits the excerpt it was given.
fn brain(n: usize, req: &Value) -> Reply {
    if n > 0 {
        return Reply::Stream { chunks: sse_reply("Made the title bigger.", &[]), delay: Duration::ZERO };
    }
    let blocks = req["messages"][0]["content"].as_array().unwrap();
    let context = blocks.iter().filter_map(|b| b["text"].as_str()).find(|t| t.contains("printed from the timeline"));
    let context = context.expect("the mention block is sent with the message");
    let rev: i64 = context.split("rev: ").nth(1).unwrap().split('`').next().unwrap().parse().unwrap();
    let line = context.lines().find(|l| l.contains("id=\"i_title\"")).unwrap().to_string();
    let edit = json!({"oldText": line, "newText": line.replace("fontSize={96}", "fontSize={140}")});
    Reply::Stream {
        chunks: sse_reply("", &[("tu_apply", "apply_timeline", json!({"rev": rev, "edits": [edit]}))]),
        delay: Duration::ZERO,
    }
}

fn run(meanwhile: Meanwhile) -> (tempfile::TempDir, Host, Vec<AgentEvent>, ChatStore, String, Arc<MockServer>) {
    let server = Arc::new(MockServer::start(brain));
    let d = tempfile::tempdir().unwrap();
    let mut host = Host { project: project(d.path()), meanwhile };
    let mentions = resolve(
        &host.project,
        &[MentionRef { kind: "item".into(), id: Some("i_title".into()), start: None, end: None }],
    )
    .mentions;
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("make the title bigger", "claude-sonnet-5-5").unwrap();
    let mut client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    client.backoff = vec![Duration::from_millis(10); 2];
    let mut events = vec![];
    run_turn_with(
        &client,
        &store,
        &chat.id,
        "make the title bigger",
        &mentions,
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    (d, host, events, store, chat.id, server)
}

fn calls(events: &[AgentEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| if let AgentEvent::ToolCall { name, .. } = e { Some(name.clone()) } else { None })
        .collect()
}

#[test]
fn a_mentioned_item_is_edited_without_printing_and_the_mention_is_logged() {
    let (_d, host, events, store, chat, server) = run(Meanwhile::Nothing);
    assert_eq!(calls(&events), ["apply_timeline"], "no print_timeline before the edit");
    let title = &host.project.doc().project.items["i_title"];
    assert_eq!(title.fields["fontSize"], 140);
    let recs = store.records(&chat);
    let logged =
        recs.iter().find_map(|r| if let Record::Mentions { mentions, .. } = r { Some(mentions) } else { None });
    assert_eq!(logged.unwrap()[0].ids, ["i_title"]);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}

#[test]
fn an_edit_after_a_concurrent_move_in_the_editor_merges_onto_the_moved_item() {
    let (_d, host, events, _store, _chat, _server) = run(Meanwhile::MoveTitle);
    assert_eq!(calls(&events), ["apply_timeline"]);
    assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolResult { ok: true, .. })));
    let title = &host.project.doc().project.items["i_title"];
    assert_eq!(title.fields["fontSize"], 140, "the assistant's edit landed");
    assert_eq!((title.start, title.end), (5_000_000, 8_000_000), "the person's move is kept");
}

#[test]
fn an_edit_to_a_mentioned_item_deleted_meanwhile_is_a_conflict_and_does_not_bring_it_back() {
    let (_d, host, events, store, chat, _server) = run(Meanwhile::DeleteTitle);
    let recs = store.records(&chat);
    assert!(
        !host.project.doc().project.items.contains_key("i_title"),
        "the deleted item is not created again: {recs:?}"
    );
    let result = store
        .records(&chat)
        .into_iter()
        .find_map(|r| if let Record::ToolResult { text, .. } = r { Some(text) } else { None })
        .unwrap();
    assert!(result.contains("conflict"), "the model is told: {result}");
    assert!(!events.is_empty());
}

#[test]
fn a_skill_chosen_from_the_slash_menu_is_read_at_the_start_of_the_turn() {
    // The app reads the skill (registry `read_skill`) when the message is sent; the turn's first request carries
    // its text ahead of any tool call, with the person's words.
    let server = MockServer::start(|_, _| Reply::Stream { chunks: sse_reply("Matched.", &[]), delay: Duration::ZERO });
    let d = tempfile::tempdir().unwrap();
    let mut host = Host { project: project(d.path()), meanwhile: Meanwhile::Nothing };
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("match these two shots", "claude-sonnet-5-5").unwrap();
    let skill =
        ove_agent::mention::skill("color-match", "Colour matching", "1. Look at both shots.\n2. Grade the target.");
    let mut client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    client.backoff = vec![Duration::from_millis(10); 2];
    run_turn_with(
        &client,
        &store,
        &chat.id,
        "match these two shots",
        &[skill],
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let reqs = server.requests.lock().unwrap();
    let first = reqs[0]["messages"][0]["content"].as_array().unwrap();
    assert_eq!(first[0]["text"], "match these two shots");
    let text = first[1]["text"].as_str().unwrap();
    assert!(text.contains("<skill id=\"color-match\">\n1. Look at both shots."), "{text}");
    assert!(text.contains("Follow it"), "{text}");
}
