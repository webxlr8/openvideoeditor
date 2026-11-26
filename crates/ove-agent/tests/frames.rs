//! The in-app assistant sees frames: a tool that returns images (here `look`) reaches the model as image blocks in
//! its tool result, in this request and when the chat is replayed (unify-agent-tool-layer task 5.3).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ove_agent::mock::{MockServer, Reply, sse_reply};
use ove_agent::tools::{ApplyRequest, ToolHost, ToolOutput};
use ove_agent::{ChatStore, Client, TurnConfig, run_turn};
use serde_json::{Map, Value, json};

/// A host whose frame tools return one picture (the rendering itself is tested in ove-tools and ove-analyze).
struct FrameHost {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl ToolHost for FrameHost {
    fn print_timeline(&mut self, _: Option<usize>, _: Option<usize>) -> Result<String, String> {
        Ok("rev: 1".into())
    }
    fn apply_timeline(&mut self, _: ApplyRequest) -> Result<String, String> {
        Err("not here".into())
    }
    fn verify_timeline(&mut self) -> Result<String, String> {
        Ok("0 findings".into())
    }
    fn list_media(&mut self) -> Result<String, String> {
        Ok("{}".into())
    }
    fn undo_last(&mut self) -> Result<String, String> {
        Err("nothing".into())
    }
    fn ask_user(&mut self, _: &str, _: &str, _: &[String]) -> Result<String, String> {
        Err("no".into())
    }
    fn run_tool(
        &mut self,
        tool: &'static ove_tools::ToolSpec,
        args: &Map<String, Value>,
    ) -> Option<Result<ToolOutput, String>> {
        self.calls.lock().unwrap().push((tool.name.to_string(), Value::Object(args.clone())));
        (tool.name == "look").then(|| {
            Ok(ToolOutput {
                text: json!({"rev": 1, "frames": [{"at": 1.5, "width": 4, "height": 2}]}).to_string(),
                images: vec![ove_tools::Png { bytes: b"\x89PNG-frame".to_vec(), width: 4, height: 2 }],
            })
        })
    }
}

#[test]
fn a_turn_that_calls_look_receives_the_frame_as_an_image_block() {
    let server = MockServer::start(|n, _req| {
        let chunks = if n == 0 {
            sse_reply("Let me look at the title.", &[("tu_look", "look", json!({"at": "1.5", "grid": true}))])
        } else {
            sse_reply("The title sits left of centre.", &[])
        };
        Reply::Stream { chunks, delay: Duration::from_millis(5) }
    });
    let d = tempfile::tempdir().unwrap();
    let store = ChatStore::for_bundle(d.path());
    let chat = store.create("check the title position", "claude-sonnet-5-5").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut host = FrameHost { calls: calls.clone() };
    let mut client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    client.backoff = vec![Duration::from_millis(10); 4];
    let out = run_turn(
        &client,
        &store,
        &chat.id,
        "check the title position",
        &mut host,
        &TurnConfig::default(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert!(out.text.contains("left of centre"));
    // The assistant's tool list offers the frame tools.
    let reqs = server.requests.lock().unwrap();
    let names: Vec<&str> = reqs[0]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for t in ["look", "grab_frame", "filmstrip", "detect_scenes", "audio_levels", "detect_silence"] {
        assert!(names.contains(&t), "{t} missing from {names:?}");
    }
    assert_eq!(calls.lock().unwrap()[0], ("look".to_string(), json!({"at": "1.5", "grid": true})));
    // The second request carries the frame inside the tool result.
    let result = reqs[1]["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "user").unwrap()["content"]
        [0]
    .clone();
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "tu_look");
    let content = result["content"].as_array().unwrap_or_else(|| panic!("{result}"));
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
    assert_eq!(content[1]["source"]["data"], ove_tools::receipt::base64(b"\x89PNG-frame"));
    drop(reqs);
    // Replaying the chat (the next turn) keeps it.
    let replayed = ove_agent::chat::messages(&store.records(&chat.id));
    let again = replayed.iter().find(|m| m["role"] == "user" && m["content"][0]["type"] == "tool_result").unwrap();
    assert_eq!(again["content"][0]["content"][1]["type"], "image");
}
