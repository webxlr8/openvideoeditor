//! add-transcript-editing 5.3: the in-app assistant has the transcript tools with the same contracts. A scripted
//! model reads the transcript once, then removes two words by their ids in two calls without reading it again.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

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
    let mut p = lib.create("Talk").unwrap();
    let ops: Vec<ove_core::ops::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addMedia", "media": {"id": "m", "data": {"type": "audio", "fileName": "talk.wav", "duration": 12, "hasAudio": true}},
         "entry": {"mediaId": "m", "nativePath": dir.join("talk.wav").to_string_lossy(), "ownedByBundle": false, "contentId": "cid"}},
        {"op": "addItem", "item": {"id": "v", "trackId": "t_main", "type": "audio", "startTime": 0, "endTime": 8, "mediaItemId": "m"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let w = |t: &str, s: f64, e: f64| ove_transcribe::Word {
        text: t.into(),
        start_us: (s * 1e6) as i64,
        end_us: (e * 1e6) as i64,
        confidence: 0.9,
    };
    let t = ove_transcribe::Transcript {
        language: "en".into(),
        model: "base".into(),
        words: vec![
            w("Hello", 0.2, 0.6),
            w("um", 0.8, 1.1),
            w("everyone.", 1.3, 1.8),
            w("We", 4.2, 4.5),
            w("really", 4.7, 5.0),
            w("edit.", 5.2, 5.8),
        ],
        duration_us: 8_000_000,
    };
    ove_transcribe::cache::store(p.bundle(), &ove_transcribe::cache::transcript_key("cid", "base", None, false), &t);
    p
}

#[test]
fn the_assistant_removes_words_by_id_from_one_transcript_read() {
    let server = MockServer::start(|n, _| {
        let chunks = match n {
            0 => sse_reply("", &[("tu_read", "get_transcript", json!({}))]),
            1 => sse_reply("", &[("tu_cut1", "remove_words", json!({"ids": ["m@800"]}))]),
            2 => sse_reply("", &[("tu_cut2", "remove_words", json!({"ids": ["m@4700"], "pause": "tight"}))]),
            _ => sse_reply("Removed the filler and 'really'.", &[]),
        };
        Reply::Stream { chunks, delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let mut host = Host { project: project(d.path()) };
    let store = ChatStore::for_bundle(host.project.bundle());
    let chat = store.create("tighten it", "claude-sonnet-5-5").unwrap();
    let before = host.project.rev();
    let mut client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    client.backoff = vec![Duration::from_millis(10); 4];
    let mut events = vec![];
    run_turn(
        &client,
        &store,
        &chat.id,
        "tighten it",
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
    assert_eq!(calls, ["get_transcript", "remove_words", "remove_words"], "one read, two cuts");
    let ok = events.iter().filter(|e| matches!(e, AgentEvent::ToolResult { ok: true, .. })).count();
    assert_eq!(ok, 3, "{events:?}");
    // The second cut used an id from the first read, after the first cut changed the timeline.
    let clips = ove_transcribe::timeline::all_clips(host.project.doc(), &|_| Some(d.path().join("talk.wav")));
    let t = ove_transcribe::cache::any_for(host.project.bundle(), "cid").remove(0);
    let (words, _) = ove_transcribe::timeline::words(&clips, &std::collections::HashMap::from([("m".to_string(), t)]));
    let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["Hello", "everyone.", "We", "edit."]);
    assert!(host.project.rev() > before);
    assert_eq!(host.project.latest_record().unwrap().unwrap().source, Source::Agent);
}
