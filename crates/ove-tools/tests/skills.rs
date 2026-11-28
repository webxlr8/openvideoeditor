//! `list_skills` and `read_skill` on the in-app and MCP surfaces, including the `skill.unknown` refusal
//! (add-agent-skills-and-connectors task 1.2), and the in-app `manage_skills` (task 2.1).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::sync::Arc;

use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use ove_tools::skills::{SKILL_FILE, SkillStore};
use ove_tools::{AppCtx, AppHost, BundleCtx, Fail, Surface, ToolCtx};
use serde_json::{Map, Value, json};

const TIDY: &str =
    "---\nname: Tidy cuts\ndescription: Close small gaps on the main track.\n---\n\nPrint, then close each gap.\n";

/// A host over one project and its library, as the desktop app provides them.
struct Host {
    project: Project,
    library: Library,
}

impl AppHost for Host {
    fn with_open(&mut self, _edit: bool, f: &mut dyn FnMut(&mut Project) -> Result<(), Fail>) -> Result<(), Fail> {
        f(&mut self.project)
    }
    fn library(&mut self) -> Option<&Library> {
        Some(&self.library)
    }
}

fn library() -> (tempfile::TempDir, Library) {
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path().join("lib"), Arc::new(ove_core::SystemClock)).unwrap();
    let store = SkillStore::of_library(&lib.root);
    std::fs::create_dir_all(store.dir("tidy-cuts")).unwrap();
    std::fs::write(store.dir("tidy-cuts").join(SKILL_FILE), TIDY).unwrap();
    std::fs::create_dir_all(store.dir("half-done")).unwrap();
    std::fs::write(store.dir("half-done").join(SKILL_FILE), "---\nname: Half\n---\nNo description.\n").unwrap();
    (d, lib)
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn check_surface(ctx: &mut dyn ToolCtx) {
    let surface = ctx.surface();
    let listed = ove_tools::call(ctx, "list_skills", &Map::new()).unwrap();
    let ids: Vec<&str> = listed.json["skills"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["caption-styling", "colour-matching", "interview-cutting", "tidy-cuts"], "{surface:?}");
    assert!(listed.text.contains("tidy-cuts\tClose small gaps"), "{}", listed.text);
    assert!(!listed.text.contains("half-done"), "invalid skills are never offered");

    let read = ove_tools::call(ctx, "read_skill", &args(json!({"id": "tidy-cuts"}))).unwrap();
    assert_eq!(read.json["text"], TIDY);
    assert!(read.text.contains("Print, then close each gap."));
    assert!(read.text.starts_with("Skill `tidy-cuts` (source: local)"), "{}", read.text);

    let unknown = ove_tools::call(ctx, "read_skill", &args(json!({"id": "nope"}))).unwrap_err();
    assert_eq!(unknown.code, "skill.unknown");
    assert_eq!(
        unknown.details["installed"],
        json!(["caption-styling", "colour-matching", "interview-cutting", "tidy-cuts"])
    );
    assert!(unknown.message.contains("tidy-cuts"));
    let invalid = ove_tools::call(ctx, "read_skill", &args(json!({"id": "half-done"}))).unwrap_err();
    assert_eq!(invalid.code, "skill.invalid");
    assert!(invalid.message.contains("description"));
}

#[test]
fn list_and_read_skills_on_the_mcp_and_pinned_surfaces() {
    let (_d, lib) = library();
    for surface in [Surface::Mcp, Surface::Pinned] {
        let mut ctx = BundleCtx::open(Some(&lib.root), surface, Source::Mcp).unwrap();
        check_surface(&mut ctx);
    }
}

#[test]
fn list_and_read_skills_in_the_app() {
    let (_d, library) = library();
    let project = library.create("Demo").unwrap();
    let mut host = Host { project, library };
    let mut ctx = AppCtx::assistant(&mut host);
    check_surface(&mut ctx);
}

#[test]
fn the_skill_tools_are_listed_on_every_agent_surface() {
    for surface in [Surface::InApp, Surface::Mcp, Surface::Pinned] {
        let names: Vec<&str> = ove_tools::tools_on(surface).map(|t| t.name).collect();
        assert!(names.contains(&"list_skills") && names.contains(&"read_skill"), "{surface:?}");
    }
}

#[test]
fn skills_follow_the_store_between_calls() {
    let (_d, lib) = library();
    let mut ctx = BundleCtx::open(Some(&lib.root), Surface::Mcp, Source::Mcp).unwrap();
    let count = |ctx: &mut BundleCtx| ove_tools::call(ctx, "list_skills", &Map::new()).unwrap().json["count"].clone();
    assert_eq!(count(&mut ctx), 4);
    let store = SkillStore::of_library(Path::new(&lib.root));
    std::fs::remove_dir_all(store.dir("tidy-cuts")).unwrap();
    assert_eq!(count(&mut ctx), 3);
}

#[test]
fn manage_skills_creates_updates_and_removes_only_the_persons_skills() {
    let (_d, library) = library();
    let project = library.create("Demo").unwrap();
    let store = SkillStore::of_library(&library.root);
    let mut host = Host { project, library };
    let mut ctx = AppCtx::assistant(&mut host);
    let mut run = |a: Value| ove_tools::call(&mut ctx, "manage_skills", &args(a));

    // Create without an id: the id comes from the name, and a taken one gets a suffix.
    let made = run(json!({"action": "create", "text": TIDY})).unwrap();
    assert_eq!(made.json["id"], "tidy-cuts-2");
    assert_eq!(made.json["source"], "created by the assistant");
    assert!(made.text.contains("Settings → Skills"));
    assert_eq!(run(json!({"action": "create", "id": "tidy-cuts", "text": TIDY})).unwrap_err().code, "skill.exists");
    assert_eq!(
        run(json!({"action": "create", "id": "caption-styling", "text": TIDY})).unwrap_err().code,
        "skill.builtin"
    );
    assert_eq!(run(json!({"action": "create", "text": "no front matter"})).unwrap_err().code, "skill.invalid");
    // Tool names that do not exist are pointed out, not refused.
    let odd = TIDY.replace("Print, then", "Run `make_magic`, then");
    let made = run(json!({"action": "create", "id": "odd", "text": odd})).unwrap();
    assert_eq!(made.json["unknownTools"], json!(["make_magic"]));

    // Update needs an existing, non-builtin skill.
    let updated = run(json!({"action": "update", "id": "tidy-cuts", "text": TIDY.replace("gap", "seam")})).unwrap();
    assert_eq!(updated.json["action"], "update");
    assert!(store.get("tidy-cuts").unwrap().text.contains("seam"));
    assert_eq!(run(json!({"action": "update", "id": "nope", "text": TIDY})).unwrap_err().code, "skill.unknown");
    assert_eq!(run(json!({"action": "update", "text": TIDY})).unwrap_err().code, "argument.missing");
    assert_eq!(
        run(json!({"action": "update", "id": "interview-cutting", "text": TIDY})).unwrap_err().code,
        "skill.builtin"
    );

    // Remove: builtin skills stay.
    assert_eq!(run(json!({"action": "remove", "id": "caption-styling"})).unwrap_err().code, "skill.builtin");
    assert!(store.get("caption-styling").is_ok());
    run(json!({"action": "remove", "id": "odd"})).unwrap();
    assert!(store.get("odd").is_err());
    assert_eq!(run(json!({"action": "remove", "id": "../lib"})).unwrap_err().code, "skill.unknown");
    assert_eq!(run(json!({"action": "rename"})).unwrap_err().code, "argument.invalid");
}

#[test]
fn manage_skills_is_not_offered_outside_the_app() {
    let (_d, lib) = library();
    let mut ctx = BundleCtx::open(Some(&lib.root), Surface::Mcp, Source::Mcp).unwrap();
    let e =
        ove_tools::call(&mut ctx, "manage_skills", &args(json!({"action": "remove", "id": "tidy-cuts"}))).unwrap_err();
    assert_eq!(e.code, "tool.unknown");
}
