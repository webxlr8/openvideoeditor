//! A mock model follows each starter skill on a sample project through the in-app tools, without a tool error
//! (add-agent-skills-and-connectors task 3.2). Every tool runs its registry handler on the project, as in the app.
//! Frame tools (`look`, `grab_frame`) answer with one stand-in picture, as in tests/frames.rs: this crate is built
//! without FFmpeg, and rendering is tested in ove-tools and ove-analyze.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ove_agent::mock::{MockServer, Reply, sse_reply};
use ove_agent::tools::{ApplyRequest, ToolHost, ToolOutput};
use ove_agent::{ChatStore, Client, TurnConfig, run_turn};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use ove_tools::{AppCtx, AppHost, Fail, Png};
use ove_transcribe::cuts::Envelope;
use ove_transcribe::vad::{CELL_US, SpeechMask};
use ove_transcribe::{Transcript, Word};
use serde_json::{Map, Value, json};

// ------------------------------------------------------------------------------------------------------------------
// The sample project: two video clips, two captions, a title and a 12 s interview recording with its cached
// transcript, speech mask and levels (speech 0–2 s, silence 2–4 s, speech 4–6 s, a music bed 6–10 s, speech 10–12 s).

fn w(t: &str, s: f64, e: f64) -> Word {
    Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: (e * 1e6) as i64, confidence: 0.9 }
}

fn sample(dir: &std::path::Path) -> (Library, Project) {
    // The files exist, so the media is online (nothing here decodes them).
    std::fs::write(dir.join("camera.mp4"), b"stand-in").unwrap();
    std::fs::write(dir.join("talk.wav"), b"stand-in").unwrap();
    let lib = Library::open(dir.join("lib"), Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = lib.create("Sample").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_cap", "name": "Captions", "order": 0, "type": "caption"}},
        {"op": "addTrack", "track": {"id": "t_text", "name": "Text", "order": 1, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 2, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_audio", "name": "Audio", "order": 3, "type": "audio"}},
        {"op": "addMedia", "media": {"id": "m_cam", "data": {"type": "video", "fileName": "camera.mp4", "duration": 12, "width": 1920, "height": 1080, "hasAudio": false}},
         "entry": {"mediaId": "m_cam", "nativePath": dir.join("camera.mp4").to_string_lossy(), "ownedByBundle": false, "contentId": "cam"}},
        {"op": "addMedia", "media": {"id": "m", "data": {"type": "audio", "fileName": "talk.wav", "duration": 12, "hasAudio": true}},
         "entry": {"mediaId": "m", "nativePath": dir.join("talk.wav").to_string_lossy(), "ownedByBundle": false, "contentId": "cid"}},
        {"op": "addItem", "item": {"id": "i_a", "trackId": "t_main", "type": "video", "startTime": 0, "endTime": 6, "mediaItemId": "m_cam", "name": "Wide"}},
        {"op": "addItem", "item": {"id": "i_b", "trackId": "t_main", "type": "video", "startTime": 6, "endTime": 12, "mediaItemId": "m_cam", "trimStart": 6, "name": "Close"}},
        {"op": "addItem", "item": {"id": "i_title", "trackId": "t_text", "type": "text", "startTime": 0, "endTime": 2, "textContent": "My first video", "name": "Title"}},
        {"op": "addItem", "item": {"id": "c1", "trackId": "t_cap", "type": "caption", "startTime": 0.2, "endTime": 1.8, "captionText": "Hello um everyone.", "captionPresetId": "classic", "captionFontSize": 30}},
        {"op": "addItem", "item": {"id": "c2", "trackId": "t_cap", "type": "caption", "startTime": 4.2, "endTime": 5.8, "captionText": "We like editing.", "captionPresetId": "classic"}},
        {"op": "addItem", "item": {"id": "v", "trackId": "t_audio", "type": "audio", "startTime": 0, "endTime": 12, "mediaItemId": "m", "name": "Interview"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, Some("sample project".into())).unwrap();
    let bundle = p.bundle().to_path_buf();
    let t = Transcript {
        language: "en".into(),
        model: "base".into(),
        words: vec![
            w("Hello", 0.2, 0.6),
            w("um", 0.8, 1.1),
            w("everyone.", 1.3, 1.8),
            w("We", 4.2, 4.5),
            w("like", 4.7, 5.0),
            w("editing.", 5.2, 5.8),
            w("Thanks.", 10.3, 10.9),
        ],
        duration_us: 12_000_000,
    };
    ove_transcribe::cache::store(&bundle, &ove_transcribe::cache::transcript_key("cid", "base", None, false), &t);
    let probs = (0..(12_000_000 / CELL_US) as usize)
        .map(|c| {
            let t = c as i64 * CELL_US;
            if t < 2_000_000 || (4_000_000..6_000_000).contains(&t) || t >= 10_000_000 { 0.9 } else { 0.05 }
        })
        .collect();
    ove_transcribe::vad::store(&bundle, "cid", &SpeechMask { cell_us: CELL_US, probs });
    let db = (0..1200)
        .map(|c| {
            let t = c as i64 * 10_000;
            if (2_000_000..4_000_000).contains(&t) {
                -90.0
            } else if (6_000_000..10_000_000).contains(&t) {
                -26.0
            } else {
                -20.0
            }
        })
        .collect();
    ove_transcribe::levels::store(&bundle, "cid", &Envelope { cell_us: 10_000, db });
    (lib, p)
}

// ------------------------------------------------------------------------------------------------------------------
// The host: every registry tool through `AppCtx` on the project, with the library's skill store.

struct Open {
    project: Project,
    library: Library,
}

impl AppHost for Open {
    fn with_open(&mut self, _edit: bool, f: &mut dyn FnMut(&mut Project) -> Result<(), Fail>) -> Result<(), Fail> {
        f(&mut self.project)
    }
    fn library(&mut self) -> Option<&Library> {
        Some(&self.library)
    }
}

struct Host {
    open: Open,
    calls: Vec<String>,
    errors: Vec<String>,
}

impl ToolHost for Host {
    fn print_timeline(&mut self, _: Option<usize>, _: Option<usize>) -> Result<String, String> {
        Err("runs through run_tool".into())
    }
    fn apply_timeline(&mut self, _: ApplyRequest) -> Result<String, String> {
        Err("runs through run_tool".into())
    }
    fn verify_timeline(&mut self) -> Result<String, String> {
        Err("runs through run_tool".into())
    }
    fn list_media(&mut self) -> Result<String, String> {
        Err("runs through run_tool".into())
    }
    fn undo_last(&mut self) -> Result<String, String> {
        Err("runs through run_tool".into())
    }
    fn ask_user(&mut self, _: &str, _: &str, options: &[String]) -> Result<String, String> {
        Ok(options.first().cloned().unwrap_or_else(|| "Go ahead.".into()))
    }
    fn run_tool(
        &mut self,
        tool: &'static ove_tools::ToolSpec,
        args: &Map<String, Value>,
    ) -> Option<Result<ToolOutput, String>> {
        self.calls.push(tool.name.to_string());
        let r = if matches!(tool.name, "look" | "grab_frame") {
            // A stand-in picture: rendering needs FFmpeg, which this crate's tests do not link.
            Ok(ToolOutput {
                text:
                    json!({"frames": [{"time": 1.0}], "stats": [{"brightness": 0.45, "black": 0.01, "clipped": 0.0}]})
                        .to_string(),
                images: vec![Png { bytes: vec![0x89, b'P', b'N', b'G'], width: 1, height: 1 }],
            })
        } else {
            let mut ctx = AppCtx::assistant(&mut self.open);
            ove_tools::call(&mut ctx, tool.name, args)
                .map(|r| ToolOutput { text: tool.model_text(&r), images: r.images })
                .map_err(|f| f.model_text())
        };
        if let Err(e) = &r {
            self.errors.push(format!("{}: {e}", tool.name));
        }
        Some(r)
    }
}

// ------------------------------------------------------------------------------------------------------------------
// The scripted model: one tool call per request, chosen from the previous tool result.

type Step = Box<dyn Fn(&str) -> (String, Value) + Send + Sync>;

fn step(f: impl Fn(&str) -> (String, Value) + Send + Sync + 'static) -> Step {
    Box::new(f)
}

fn call(name: &str, input: Value) -> Step {
    let name = name.to_string();
    step(move |_| (name.clone(), input.clone()))
}

fn last_result(req: &Value) -> String {
    let Some(last) = req["messages"].as_array().and_then(|m| m.last()) else { return String::new() };
    let Some(b) = last["content"].as_array().and_then(|b| b.iter().rev().find(|b| b["type"] == "tool_result")) else {
        return String::new();
    };
    match &b["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

fn rev_of(printed: &str) -> i64 {
    printed
        .lines()
        .find_map(|l| l.trim().trim_start_matches('*').trim().strip_prefix("rev:").map(|r| r.trim().to_string()))
        .and_then(|r| r.parse().ok())
        .unwrap_or_else(|| panic!("no rev in {printed}"))
}

/// The printed element with this id (one or several lines).
fn element(printed: &str, id: &str) -> String {
    let lines: Vec<&str> = printed.lines().collect();
    let needle = format!("id=\"{id}\"");
    let at = lines.iter().position(|l| l.contains(&needle)).unwrap_or_else(|| panic!("no {id} in {printed}"));
    let start = (0..=at).rev().find(|&i| lines[i].trim_start().starts_with('<')).unwrap();
    let end = (at..lines.len()).find(|&i| lines[i].trim_end().ends_with("/>")).unwrap();
    lines[start..=end].join("\n")
}

/// Runs a scripted turn that follows `skill` and returns the host.
fn follow(skill: &str, steps: Vec<Step>) -> (tempfile::TempDir, Host) {
    let steps = Arc::new(steps);
    let script = steps.clone();
    let server = MockServer::start(move |_, req| {
        let done =
            req["messages"].as_array().map(|m| m.iter().filter(|x| x["role"] == "assistant").count()).unwrap_or(0);
        let chunks = match script.get(done) {
            Some(s) => {
                let (name, input) = s(&last_result(req));
                sse_reply("", &[(&format!("tu_{done}"), &name, input)])
            }
            None => sse_reply("Done, following the skill.", &[]),
        };
        Reply::Stream { chunks, delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let (library, project) = sample(d.path());
    let chats = ChatStore::for_bundle(project.bundle());
    let chat = chats.create(skill, "claude-sonnet-5-5").unwrap();
    let store = ove_tools::skills::SkillStore::of_library(&library.root);
    let mut host = Host { open: Open { project, library }, calls: Vec::new(), errors: Vec::new() };
    let client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    let cfg = TurnConfig { skill_index: ove_tools::skills::index_for(&store), ..TurnConfig::default() };
    let out = run_turn(
        &client,
        &chats,
        &chat.id,
        &format!("Use the {skill} skill."),
        &mut host,
        &cfg,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(out.text, "Done, following the skill.");
    let reqs = server.requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), steps.len() + 1, "{skill}: one request per step");
    // The index the model saw names the skill.
    assert!(reqs[0]["system"][1]["text"].as_str().unwrap().contains(&format!("- {skill}:")));
    (d, host)
}

#[test]
fn the_caption_styling_skill_runs_without_tool_errors() {
    let (_d, host) = follow(
        "caption-styling",
        vec![
            call("read_skill", json!({"id": "caption-styling"})),
            call("print_timeline", json!({})),
            call("look", json!({"count": 4, "grid": true})),
            call("print_timeline", json!({})),
            step(|printed| {
                // One style for every caption, with the size override removed: one apply, one undo step.
                let edits: Vec<Value> = ["c1", "c2"]
                    .iter()
                    .map(|id| {
                        let old = element(printed, id);
                        let new: String = old
                            .replace("\"classic\"", "\"minimal-modern\"")
                            .lines()
                            .map(|l| l.replace(" captionFontSize={30}", ""))
                            .filter(|l| l.trim() != "captionFontSize={30}")
                            .collect::<Vec<_>>()
                            .join("\n");
                        json!({"oldText": old, "newText": new})
                    })
                    .collect();
                ("apply_timeline".into(), json!({"rev": rev_of(printed), "edits": edits}))
            }),
            call("look", json!({"at": "1,5", "grid": true})),
            call("verify_timeline", json!({})),
        ],
    );
    assert_eq!(host.errors, Vec::<String>::new());
    let doc = host.open.project.doc();
    for id in ["c1", "c2"] {
        assert_eq!(doc.project.items[id].fields["captionPresetId"], "minimal-modern");
    }
    assert!(doc.project.items["c1"].fields.get("captionFontSize").is_none());
}

#[test]
fn the_colour_matching_skill_runs_without_tool_errors() {
    let (_d, host) = follow(
        "colour-matching",
        vec![
            call("read_skill", json!({"id": "colour-matching"})),
            call("print_timeline", json!({})),
            call("list_media", json!({})),
            call("ask_user", json!({"question": "Which shot is your favourite?", "options": ["Wide", "Close"]})),
            call("look", json!({"at": "3,9", "stats": true})),
            call("print_timeline", json!({})),
            step(|printed| {
                // Normalise the close shot towards the wide one (exposure, then white balance).
                let old = element(printed, "i_b");
                let new = old.replacen("/>", "colorGrading={{exposure: 0.15, temperature: -6, tint: 2}} />", 1);
                ("apply_timeline".into(), json!({"rev": rev_of(printed), "edits": [{"oldText": old, "newText": new}]}))
            }),
            call("look", json!({"at": "3,9", "stats": true})),
            call("print_timeline", json!({})),
            step(|printed| {
                // The look on an adjustment item above the graded clips.
                let first_track = printed.lines().find(|l| l.trim_start().starts_with("<track ")).unwrap();
                let look = "    <track name=\"Look\" kind=\"adjustment\">\n      <adjustment name=\"Warm look\" start={0} end={12} colorGrade={{lut: {preset: \"warm\", intensity: 0.4}}} />\n    </track>\n";
                (
                    "apply_timeline".into(),
                    json!({"rev": rev_of(printed), "edits": [{"oldText": first_track, "newText": format!("{look}{first_track}")}]}),
                )
            }),
            call("look", json!({"count": 4, "stats": true})),
            call("verify_timeline", json!({})),
        ],
    );
    assert_eq!(host.errors, Vec::<String>::new());
    let doc = host.open.project.doc();
    assert_eq!(doc.project.items["i_b"].fields["colorGrading"]["exposure"], 0.15);
    let adj = doc.project.items.values().find(|i| i.kind == "adjustment").expect("the look item");
    assert_eq!(adj.fields["colorGrade"]["lut"]["preset"], "warm");
}

#[test]
fn the_interview_cutting_skill_runs_without_tool_errors() {
    let (_d, host) = follow(
        "interview-cutting",
        vec![
            call("read_skill", json!({"id": "interview-cutting"})),
            call("print_timeline", json!({})),
            call("get_transcript", json!({})),
            call("remove_silence", json!({"apply": false})),
            call("remove_silence", json!({})),
            call("find_fillers", json!({})),
            step(|found| {
                let v: Value = serde_json::from_str(found).unwrap();
                let ids: Vec<Value> = v["fillers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|f| f["selected"] == true && f["kind"] == "word")
                    .map(|f| f["id"].clone())
                    .collect();
                assert!(!ids.is_empty(), "{found}");
                ("remove_words".into(), json!({"ids": ids, "pause": "balanced"}))
            }),
            call("get_transcript", json!({})),
            call("look", json!({"at": "1,3"})),
            call("verify_timeline", json!({})),
        ],
    );
    assert_eq!(host.errors, Vec::<String>::new());
    // The dead air is gone, the hesitation is cut, the sentence ends and the context word stay.
    assert!(host.calls.contains(&"remove_words".to_string()));
    let printed = ove_agent::tools::print(&host.open.project, None, None);
    let mut ctx_host = host.open;
    let mut ctx = AppCtx::assistant(&mut ctx_host);
    let t = ove_tools::call(&mut ctx, "get_transcript", &Map::new()).unwrap();
    let words: Vec<&str> = t.json["words"].as_array().unwrap().iter().map(|w| w["text"].as_str().unwrap()).collect();
    assert_eq!(words, ["Hello", "everyone.", "We", "like", "editing.", "Thanks."], "{printed}");
}
