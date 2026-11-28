//! add-markers-keyframe-lanes-and-versions 2.4: the review workflow in the in-app assistant. A scripted model lists
//! the open markers assigned to it, checks its edit with `look`, sets the marker to review, then tries to resolve it:
//! the resolve is refused with `marker.resolveByAgent` and the marker stays in review. The rule is in the prompt.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test"
)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ove_agent::mock::{MockServer, Reply, sse_reply};
use ove_agent::tools::{self, ApplyRequest, ToolHost, ToolOutput};
use ove_agent::{AgentEvent, ChatStore, Client, TurnConfig, run_turn};
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use serde_json::{Map, Value, json};

struct Host {
    project: Project,
}

impl ToolHost for Host {
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
    fn ask_user(&mut self, _: &str, _: &str, _: &[String]) -> Result<String, String> {
        Err("no".into())
    }
    fn run_tool(
        &mut self,
        tool: &'static ove_tools::ToolSpec,
        args: &Map<String, Value>,
    ) -> Option<Result<ToolOutput, String>> {
        if tool.name == "look" {
            // Frames need the renderer; a stand-in frame is enough for the workflow.
            return Some(Ok(ToolOutput {
                text: json!({"rev": self.project.rev(), "frames": [{"at": 5, "width": 4, "height": 2}]}).to_string(),
                images: vec![ove_tools::Png { bytes: b"\x89PNG-frame".to_vec(), width: 4, height: 2 }],
            }));
        }
        let mut host = ove_tools::ProjectHost { project: &mut self.project };
        let mut ctx = ove_tools::AppCtx::assistant(&mut host);
        Some(
            ove_tools::call(&mut ctx, tool.name, args)
                .map(|r| ToolOutput { text: tool.model_text(&r), images: r.images })
                .map_err(|f| f.model_text()),
        )
    }
}

fn project(dir: &std::path::Path) -> Project {
    let lib = Library::open(dir, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = lib.create("Review").unwrap();
    let ops: Vec<ove_core::ops::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "i_title", "trackId": "t_main", "type": "text", "startTime": 0, "endTime": 10, "textContent": "Hi"}},
        {"op": "addMarker", "marker": {"id": "m1", "at": 5, "assignee": "agent", "note": "Make the title bigger"}},
        {"op": "addMarker", "marker": {"id": "m2", "at": 8, "assignee": "user"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p
}

#[test]
fn the_assistant_sets_review_after_look_and_cannot_resolve() {
    assert!(ove_agent::prompt::SYSTEM_PROMPT.contains("Never set a marker to `resolved`"));
    let server = MockServer::start(|n, _| {
        let chunks = match n {
            0 => sse_reply("", &[("tu_list", "list_markers", json!({"status": "open", "assignee": "agent"}))]),
            1 => sse_reply("", &[("tu_look", "look", json!({"at": "5"}))]),
            2 => sse_reply("", &[("tu_review", "update_marker", json!({"id": "m1", "status": "review"}))]),
            3 => sse_reply("", &[("tu_resolve", "update_marker", json!({"id": "m1", "status": "resolved"}))]),
            _ => sse_reply("The title marker is ready for your review.", &[]),
        };
        Reply::Stream { chunks, delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = Host { project: project(d.path()) };
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("work through my notes", "claude-sonnet-5-5").unwrap();
    let mut client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    client.backoff = vec![Duration::from_millis(10); 4];
    let mut events = vec![];
    run_turn(
        &client,
        &store,
        &chat.id,
        "work through my notes",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    let calls: Vec<String> = events
        .iter()
        .filter_map(|e| if let AgentEvent::ToolCall { name, .. } = e { Some(name.clone()) } else { None })
        .collect();
    assert_eq!(calls, ["list_markers", "look", "update_marker", "update_marker"]);
    let results: Vec<(bool, String)> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult { ok, summary, .. } => Some((*ok, summary.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(results.iter().map(|r| r.0).collect::<Vec<_>>(), [true, true, true, false], "{results:?}");

    // The listing gave only the agent's open marker, with the item under it.
    let reqs = server.requests.lock().unwrap();
    let listed = reqs[1]["messages"].as_array().unwrap().last().unwrap()["content"][0]["content"].to_string();
    assert!(listed.contains("m1") && listed.contains("i_title") && !listed.contains("m2"), "{listed}");
    // The model saw the refusal code.
    let refused = reqs[4]["messages"].as_array().unwrap().last().unwrap()["content"][0].to_string();
    assert!(refused.contains("marker.resolveByAgent"), "{refused}");
    drop(reqs);

    let m = host.project.doc().marker("m1").unwrap().clone();
    assert_eq!(m.status, "review", "the refused resolve left the marker in review");
    assert_eq!(host.project.latest_record().unwrap().unwrap().source, Source::Agent);
}
