//! Skills in the in-app assistant (add-agent-skills-and-connectors): the skill index as a second cached system block
//! that leaves the fixed prompt byte-identical (task 1.3), `manage_skills` driven by a mock model (task 2.1), and the
//! starter skills followed on a sample project without tool errors (task 3.2).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ove_agent::agent::{request_body, system_blocks};
use ove_agent::chat::estimate_tokens;
use ove_agent::mock::{MockServer, Reply, sse_reply};
use ove_agent::tools::{ApplyRequest, ToolHost, ToolOutput};
use ove_agent::{ChatStore, Client, TurnConfig, run_turn};
use ove_store::Library;
use ove_store::project::Project;
use ove_tools::skills::{SKILL_FILE, SkillStore, index_for};
use ove_tools::{AppCtx, AppHost, Fail};
use serde_json::{Map, Value};

fn skill(i: usize) -> String {
    format!("---\nname: Skill {i}\ndescription: Playbook number {i} for a kind of edit.\n---\n\nStep {i}.\n")
}

fn store_with(lib: &std::path::Path, n: usize) -> SkillStore {
    let s = SkillStore::of_library(lib);
    for i in 0..n {
        let id = format!("skill-{i:02}");
        std::fs::create_dir_all(s.dir(&id)).unwrap();
        std::fs::write(s.dir(&id).join(SKILL_FILE), skill(i)).unwrap();
    }
    s
}

#[test]
fn the_fixed_prompt_and_tools_are_byte_identical_with_0_and_20_skills() {
    let d = tempfile::tempdir().unwrap();
    let none = TurnConfig::default();
    let store = store_with(d.path(), 20);
    let index = index_for(&store).unwrap();
    assert_eq!(index.lines().filter(|l| l.starts_with("- ")).count(), 23, "20 skills and the 3 builtin ones");
    let twenty = TurnConfig { skill_index: Some(index.clone()), ..TurnConfig::default() };
    let a = request_body(&none, &[]);
    let b = request_body(&twenty, &[]);
    assert_eq!(a["system"].as_array().unwrap().len(), 1);
    assert_eq!(b["system"].as_array().unwrap().len(), 2);
    assert_eq!(a["system"][0].to_string(), b["system"][0].to_string(), "the fixed block is byte-identical");
    assert_eq!(a["system"][0]["text"], ove_agent::prompt::SYSTEM_PROMPT);
    assert_eq!(a["tools"].to_string(), b["tools"].to_string());
    // The index has its own breakpoint, so a change to it does not touch the fixed block's cache entry.
    assert_eq!(b["system"][1]["text"], index);
    assert_eq!(b["system"][1]["cache_control"]["type"], "ephemeral");
    assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
    // An empty index sends no second block.
    let empty = TurnConfig { skill_index: Some("  ".into()), ..TurnConfig::default() };
    assert_eq!(system_blocks(&empty).as_array().unwrap().len(), 1);
}

/// Splits a request into the parts the prompt cache sees in order: tools, each system block, each message.
fn parts(req: &Value) -> Vec<String> {
    let mut out = vec![req["tools"].to_string()];
    out.extend(req["system"].as_array().unwrap().iter().map(Value::to_string));
    out.extend(req["messages"].as_array().unwrap().iter().map(Value::to_string));
    out
}

/// Of the input of `req`, the share (by estimated tokens) that repeats `prev` from the start: what a prefix cache
/// can serve. The last message is always new.
fn reused(prev: &[String], req: &[String]) -> (u64, u64) {
    let tokens = |s: &String| estimate_tokens(&Value::String(s.clone()));
    let total: u64 = req.iter().map(tokens).sum();
    let same: u64 = req.iter().zip(prev).take_while(|(a, b)| a == b).map(|(a, _)| tokens(a)).sum();
    (same, total)
}

/// Ten turns with 20 skills installed; one more skill is installed before turn 6. Records the modelled prompt-cache
/// reuse (design.md, Results). A live measurement needs a real API key.
#[test]
fn the_cached_prefix_survives_ten_turns_and_a_skill_install() {
    let server = MockServer::start(|_, _| Reply::Stream { chunks: sse_reply("Done.", &[]), delay: Duration::ZERO });
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
    let store = store_with(d.path(), 20);
    let p = lib.create("Demo").unwrap();
    let chats = ChatStore::for_bundle(p.bundle());
    let chat = chats.create("hi", "claude-sonnet-5-5").unwrap();
    let mut host = Host::new(p, lib.clone());
    let client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    for turn in 0..10 {
        if turn == 5 {
            std::fs::create_dir_all(store.dir("late")).unwrap();
            std::fs::write(store.dir("late").join(SKILL_FILE), skill(99)).unwrap();
        }
        let cfg = TurnConfig { skill_index: index_for(&store), ..TurnConfig::default() };
        let text = format!("turn {turn}");
        run_turn(&client, &chats, &chat.id, &text, &mut host, &cfg, &AtomicBool::new(false), &mut |_| {}).unwrap();
    }
    let reqs = server.requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 10);
    let fixed: Vec<String> = reqs.iter().map(|r| r["system"][0].to_string()).collect();
    assert!(fixed.windows(2).all(|w| w[0] == w[1]), "the fixed block never changes");
    let (mut same, mut total) = (0, 0);
    for w in reqs.windows(2) {
        let (s, t) = reused(&parts(&w[0]), &parts(&w[1]));
        same += s;
        total += t;
    }
    let rate = same as f64 / total as f64;
    // Turn 6 misses from the index on; every other turn reuses everything but its new message.
    let turn6 = reused(&parts(&reqs[4]), &parts(&reqs[5]));
    let tools_and_fixed: u64 = parts(&reqs[5])[..2].iter().map(|s| estimate_tokens(&Value::String(s.clone()))).sum();
    assert_eq!(turn6.0, tools_and_fixed, "after an install, the tools and the fixed prompt still hit");
    eprintln!(
        "modelled prompt-cache reuse over turns 2-10 with 20 skills (one installed before turn 6): {:.1} %",
        rate * 100.0
    );
    assert!(rate > 0.9, "{rate}");
    // The index in the sixth request lists the new skill.
    assert!(reqs[5]["system"][1]["text"].as_str().unwrap().contains("- late:"));
}

// ------------------------------------------------------------------------------------------------------------------
// A host that runs every registry tool through `AppCtx`, as the desktop app does, with the library's skill store.

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

pub struct Host {
    open: Open,
    pub errors: Vec<String>,
}

impl Host {
    pub fn new(project: Project, library: Library) -> Host {
        Host { open: Open { project, library }, errors: Vec::new() }
    }
}

impl ToolHost for Host {
    fn print_timeline(&mut self, _: Option<usize>, _: Option<usize>) -> Result<String, String> {
        unreachable_tool()
    }
    fn apply_timeline(&mut self, _: ApplyRequest) -> Result<String, String> {
        unreachable_tool()
    }
    fn verify_timeline(&mut self) -> Result<String, String> {
        unreachable_tool()
    }
    fn list_media(&mut self) -> Result<String, String> {
        unreachable_tool()
    }
    fn undo_last(&mut self) -> Result<String, String> {
        unreachable_tool()
    }
    fn ask_user(&mut self, _: &str, _: &str, options: &[String]) -> Result<String, String> {
        Ok(options.first().cloned().unwrap_or_else(|| "Go ahead.".into()))
    }
    fn run_tool(
        &mut self,
        tool: &'static ove_tools::ToolSpec,
        args: &Map<String, Value>,
    ) -> Option<Result<ToolOutput, String>> {
        let mut ctx = AppCtx::assistant(&mut self.open);
        let r = ove_tools::call(&mut ctx, tool.name, args)
            .map(|r| ToolOutput { text: tool.model_text(&r), images: r.images })
            .map_err(|f| f.model_text());
        if let Err(e) = &r {
            self.errors.push(format!("{}: {e}", tool.name));
        }
        Some(r)
    }
}

fn unreachable_tool() -> Result<String, String> {
    Err("every tool runs through run_tool here".into())
}

const SAVED: &str = "---\nname: Tidy cuts\ndescription: Close small gaps on the main track.\n---\n\n1. Print the timeline.\n2. Close each gap.\n";

/// The assistant saves a workflow as a skill, updates it, and is refused when it tries to remove a builtin one.
#[test]
fn the_assistant_manages_skills_and_cannot_touch_builtin_ones() {
    let server = MockServer::start(|n, _| {
        let calls: Vec<(&str, &str, Value)> = match n {
            0 => vec![("tu_1", "manage_skills", serde_json::json!({"action": "create", "text": SAVED}))],
            1 => vec![(
                "tu_2",
                "manage_skills",
                serde_json::json!({"action": "update", "id": "tidy-cuts", "text": SAVED.replace("each gap", "every gap")}),
            )],
            2 => vec![("tu_3", "manage_skills", serde_json::json!({"action": "remove", "id": "caption-styling"}))],
            _ => vec![],
        };
        Reply::Stream { chunks: sse_reply(if calls.is_empty() { "Saved." } else { "" }, &calls), delay: Duration::ZERO }
    });
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
    let p = lib.create("Demo").unwrap();
    let chats = ChatStore::for_bundle(p.bundle());
    let chat = chats.create("save it", "claude-sonnet-5-5").unwrap();
    let mut host = Host::new(p, lib.clone());
    let client = Client::with_base_url("sk-test-key-0123456789abcdef", &server.url);
    let cfg = TurnConfig::default();
    let out = run_turn(
        &client,
        &chats,
        &chat.id,
        "Save this as a skill",
        &mut host,
        &cfg,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(out.text, "Saved.");
    let store = SkillStore::of_library(&lib.root);
    let saved = store.get("tidy-cuts").unwrap();
    assert_eq!(saved.origin, ove_tools::skills::Origin::Assistant);
    assert!(saved.text.contains("every gap"));
    assert!(!saved.modified);
    assert_eq!(host.errors.len(), 1, "{:?}", host.errors);
    assert!(host.errors[0].contains("skill.builtin"), "{:?}", host.errors);
    assert!(store.get("caption-styling").unwrap().builtin);
    // The tool is offered only in the app.
    assert!(ove_tools::find("manage_skills").unwrap().on(ove_tools::Surface::InApp));
    assert!(!ove_tools::find("manage_skills").unwrap().on(ove_tools::Surface::Mcp));
}
