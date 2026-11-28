//! `ove skills draft` (add-agent-skills-and-connectors task 3.1): the request a mock assistant receives carries the
//! timeline, the history and the presets; without an assistant, drafting stops and the context pack is still saved.

use std::sync::Arc;
use std::time::Duration;

use ove_agent::mock::{MockServer, Reply, sse_reply};
use ove_core::ops::Op;
use ove_core::revision::{Actor, ActorKind, Source};
use serde_json::json;

use super::*;

fn sample() -> (tempfile::TempDir, ove_store::project::Project) {
    let d = tempfile::tempdir().unwrap();
    let lib = ove_store::Library::open(d.path().join("lib"), Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = lib.create("Finished cut").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_cap", "name": "Captions", "order": 0, "type": "caption"}},
        {"op": "addItem", "item": {"id": "c1", "trackId": "t_cap", "type": "caption", "startTime": 0, "endTime": 2, "captionText": "Hello", "captionPresetId": "butter-serif"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, Some("captions".into())).unwrap();
    let ops: Vec<Op> =
        serde_json::from_value(json!([{"op": "updateItem", "id": "c1", "set": {"captionText": "Hello there"}}]))
            .unwrap();
    let actor =
        Actor { kind: ActorKind::Mcp, session: Some("s".into()), turn: None, client: Some("test-agent".into()) };
    p.apply_as(ops, Source::Mcp, Some("longer caption".into()), Some(actor)).unwrap();
    (d, p)
}

const DRAFT: &str = "---\nname: Warm captions\ndescription: Caption a talk in a calm serif style.\n---\n\n## Rebuild\n1. Set captionPresetId to butter-serif.\n\n## Reuse\n1. Keep the style.\n";

#[test]
fn the_draft_request_carries_the_timeline_history_and_presets() {
    let server = MockServer::start(|_, _| Reply::Stream { chunks: sse_reply(DRAFT, &[]), delay: Duration::ZERO });
    let (d, p) = sample();
    let pack = ove_tools::skills::draft::pack(&p, Some("User: make the captions calmer".into())).unwrap();
    let out = d.path().join("draft.md");
    let client = ove_agent::Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    let (v, text) = draft(&pack, Some(&out), None, Some(client), "claude-sonnet-5-5").unwrap();
    assert!(text.contains("Nothing was saved"), "{text}");
    let saved = std::fs::read_to_string(&out).unwrap();
    assert!(saved.contains("source: drafted"), "{saved}");
    assert_eq!(v["draft"], saved);
    // Nothing reaches the store.
    assert!(!d.path().join("lib").join(".skills").exists());

    let reqs = server.requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    let system = reqs[0]["system"].as_str().unwrap();
    assert!(system.contains("Rebuild") && system.contains("Reuse"));
    let user = reqs[0]["messages"][0]["content"].as_str().unwrap();
    assert!(user.contains("## Final timeline") && user.contains("<timeline"), "timeline");
    assert!(user.contains("captionPresetId=\"butter-serif\""), "the printed timeline");
    assert!(user.contains("### mcp (test-agent)") && user.contains("longer caption"), "history by actor");
    assert!(user.contains("### gui") && user.contains(" (gui): captions"), "history by actor");
    assert!(user.contains("- caption style: butter-serif (1)"), "presets");
    assert!(user.contains("User: make the captions calmer"), "chat");
}

#[test]
fn without_an_assistant_drafting_stops_and_the_pack_is_saved() {
    let (d, p) = sample();
    let pack = ove_tools::skills::draft::pack(&p, None).unwrap();
    let pack_out = d.path().join("pack.md");
    let e = draft(&pack, None, Some(&pack_out), None, "claude-sonnet-5-5").unwrap_err();
    assert_eq!(e.code, "draft.noAssistant");
    assert!(e.message.contains("Drafting needs the assistant"));
    assert!(e.message.contains("The context pack was saved to"));
    let saved = std::fs::read_to_string(&pack_out).unwrap();
    assert!(saved.contains("## Final timeline") && saved.contains("## Edit history by actor"));
    // Without --pack-out the message says how to keep the pack.
    let e = draft(&pack, None, None, None, "claude-sonnet-5-5").unwrap_err();
    assert!(e.message.contains("--pack-out"));
}

#[test]
fn a_draft_the_assistant_got_wrong_is_refused() {
    let server =
        MockServer::start(|_, _| Reply::Stream { chunks: sse_reply("Sorry, no.", &[]), delay: Duration::ZERO });
    let (_d, p) = sample();
    let pack = ove_tools::skills::draft::pack(&p, None).unwrap();
    let client = ove_agent::Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    let e = draft(&pack, None, None, Some(client), "claude-sonnet-5-5").unwrap_err();
    assert_eq!(e.code, "draft.invalid");
}
