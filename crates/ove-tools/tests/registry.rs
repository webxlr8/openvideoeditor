//! The registry on every context: each context runs the same handlers on a fixture bundle, the tool lists per
//! surface are pinned by golden files, and the same input gives the same receipt in-app and headless.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use ove_tools::{AppCtx, BundleCtx, LinkCtx, ProjectHost, Receipt, Surface, ToolCtx};
use serde_json::{Map, Value, json};

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

/// A library with one project, "Demo": a Main track with a title.
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "b", "trackId": "t_main", "type": "text", "startTime": 2, "endTime": 4, "textContent": "Hi", "name": "Title"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p.flush_projection();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn call(ctx: &mut dyn ToolCtx, name: &str, a: Value) -> Receipt {
    ove_tools::call(ctx, name, &args(a)).unwrap_or_else(|f| panic!("{name}: {f:?}"))
}

/// Drops values that depend on the clock (revisions) so receipts from two runs compare.
fn timeless(mut v: Value) -> Value {
    match &mut v {
        Value::Object(m) => {
            for k in ["rev", "baseRev", "updatedAt", "createdAt", "bundle", "id"] {
                m.remove(k);
            }
            for x in m.values_mut() {
                *x = timeless(x.take());
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                *x = timeless(x.take());
            }
        }
        Value::String(s) if s.contains("rev:") => *s = s.lines().filter(|l| !l.contains("rev:")).collect(),
        _ => {}
    }
    v
}

#[test]
fn each_context_runs_a_read_only_tool_on_a_fixture_bundle() {
    let (_d, lib, bundle) = fixture();
    let read = json!({"project": "Demo"});

    let mut b = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let from_bundle = call(&mut b, "show_project", read.clone());
    assert_eq!(from_bundle.json["name"], "Demo");
    assert_eq!(from_bundle.json["items"], 1);

    let mut l = LinkCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let from_link = call(&mut l, "show_project", read.clone());
    assert_eq!(from_link, from_bundle);

    // The app's context works on the open project; `project` may name it or be left out.
    let mut p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    let mut host = ProjectHost { project: &mut p };
    let mut app = AppCtx { host: &mut host, source: Source::Agent, surface: Surface::Mcp, actor: None };
    assert_eq!(call(&mut app, "show_project", read), from_bundle);
    assert_eq!(call(&mut app, "show_project", json!({})), from_bundle);
    let other = ove_tools::call(&mut app, "show_project", &args(json!({"project": "Other"}))).unwrap_err();
    assert_eq!(other.code, "project.notOpen");
    let printed = call(&mut app, "print_timeline", json!({}));
    assert!(printed.json["text"].as_str().unwrap().contains("name=\"Title\""));
}

#[test]
fn the_same_edit_gives_the_same_receipt_in_app_and_on_disk() {
    let edit = |text: &str| {
        let line = text.lines().find(|l| l.contains("name=\"Title\"")).unwrap();
        json!({"edits": [{"oldText": line, "newText": line.replace("Title", "Opening title")}]})
    };
    // Headless, through the command line's context.
    let (_d1, lib1, _) = fixture();
    let mut b = BundleCtx::open(Some(&lib1), Surface::Cli, Source::Cli).unwrap();
    let text = call(&mut b, "print_timeline", json!({"project": "Demo"})).json["text"].as_str().unwrap().to_string();
    let rev = call(&mut b, "show_project", json!({"project": "Demo"})).json["rev"].clone();
    let mut a = edit(&text);
    a["project"] = json!("Demo");
    a["rev"] = rev;
    let on_disk = call(&mut b, "apply_timeline", a);

    // In the app, on its open project.
    let (_d2, _lib2, bundle2) = fixture();
    let mut p = Project::open(&bundle2, Mode::Write, clock()).unwrap();
    let mut host = ProjectHost { project: &mut p };
    let mut app = AppCtx::assistant(&mut host);
    let printed = call(&mut app, "print_timeline", json!({}));
    let mut a = edit(printed.json["text"].as_str().unwrap());
    a["rev"] = printed.json["rev"].clone();
    let in_app = call(&mut app, "apply_timeline", a);

    assert_eq!(on_disk.json["changed"], true, "{}", on_disk.json);
    assert_eq!(timeless(on_disk.json.clone()), timeless(in_app.json.clone()));
    assert_eq!(p.doc().item("b").unwrap().get("name"), Some(json!("Opening title")));
    assert_eq!(p.latest_record().unwrap().unwrap().source, Source::Agent);
}

fn golden(name: &str, v: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let text = format!("{}\n", serde_json::to_string_pretty(v).unwrap());
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; run with UPDATE_GOLDEN=1", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(text, want, "{name} changed: tool names, order or text are pinned (UPDATE_GOLDEN=1 rewrites them)");
}

#[test]
fn tool_lists_per_surface_are_pinned_and_stable() {
    // The in-app list is the prompt-cache prefix: byte-identical across calls and pinned by a golden file.
    let in_app = Value::Array(ove_tools::anthropic_list());
    assert_eq!(
        serde_json::to_string(&in_app).unwrap(),
        serde_json::to_string(&Value::Array(ove_tools::anthropic_list())).unwrap()
    );
    golden("tools-inApp.json", &in_app);
    golden("tools-mcp.json", &ove_tools::mcp_list(Surface::Mcp));
    golden("tools-pinned.json", &ove_tools::mcp_list(Surface::Pinned));
}

#[test]
fn every_tool_has_one_definition_and_a_valid_schema() {
    let mut names = std::collections::BTreeSet::new();
    for t in ove_tools::TOOLS {
        for n in std::iter::once(&t.name).chain(t.aliases) {
            assert!(names.insert(*n), "{n} is defined twice");
        }
        let s = t.schema();
        assert_eq!(s["type"], "object");
        for (k, p) in s["properties"].as_object().unwrap() {
            assert!(p.get("type").is_some(), "{}.{k} has no type: {p}", t.name);
        }
        assert!(t.surfaces != 0, "{} appears nowhere", t.name);
    }
    // Same name, same schema on every surface.
    let mcp = ove_tools::mcp_list(Surface::Mcp);
    for t in ove_tools::anthropic_list() {
        if let Some(m) = mcp["tools"].as_array().unwrap().iter().find(|m| m["name"] == t["name"]) {
            assert_eq!(m["inputSchema"], t["input_schema"], "{}", t["name"]);
            assert_eq!(m["description"], t["description"], "{}", t["name"]);
        }
    }
}

#[test]
fn unknown_tools_and_bad_arguments_are_refused_with_codes() {
    let (_d, lib, _) = fixture();
    let mut b = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let e = ove_tools::call(&mut b, "print_timelines", &Map::new()).unwrap_err();
    assert_eq!(e.code, "tool.unknown");
    assert_eq!(e.details["suggestions"][0], "print_timeline");
    // In-app only.
    assert_eq!(ove_tools::call(&mut b, "ask_user", &args(json!({"question": "?"}))).unwrap_err().code, "tool.unknown");
    // Former names still work.
    assert!(ove_tools::find("undo_last").is_some_and(|t| t.name == "undo_timeline"));
    assert!(ove_tools::find("seek_viewer").is_some_and(|t| t.name == "seek"));
    assert!(ove_tools::find("import_media").is_some_and(|t| t.name == "add_media"));
    let mut code = |a: Value| ove_tools::call(&mut b, "print_timeline", &args(a)).unwrap_err().code;
    assert_eq!(code(json!({"project": "Demo", "nope": 1})), "argument.unknown");
    assert_eq!(code(json!({"project": "Demo", "from": "x"})), "argument.invalid");
    assert_eq!(code(json!({})), "argument.missing");
    // Command-line-only arguments are not for agents.
    let e = ove_tools::call(&mut b, "look", &args(json!({"project": "Demo", "out": "x"}))).unwrap_err();
    assert_eq!(e.code, "argument.unknown");
}

#[test]
fn the_tool_docs_list_every_tool() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli/tools.md");
    let doc = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let (head, rest) = doc.split_once("<!-- tools:begin -->\n").unwrap();
    let (_, tail) = rest.split_once("<!-- tools:end -->").unwrap();
    let want = format!("{head}<!-- tools:begin -->\n{}<!-- tools:end -->{tail}", ove_tools::docs_table());
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    assert_eq!(doc, want, "docs/cli/tools.md is out of date (UPDATE_GOLDEN=1 rewrites its table)");
}

#[test]
fn the_assistant_runs_edit_timeline_on_the_open_project() {
    assert!(ove_tools::anthropic_list().iter().any(|t| t["name"] == "edit_timeline"));
    let (_d, _lib, bundle) = fixture();
    let mut p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    let mut host = ProjectHost { project: &mut p };
    let mut app = AppCtx::assistant(&mut host);
    let r = call(&mut app, "edit_timeline", json!({"request": {"op": "split", "items": ["b"], "at": 3}}));
    assert_eq!(r.json["verb"], "split", "{}", r.json);
    assert_eq!(r.json["changedItems"].as_array().unwrap().len(), 2, "{}", r.json);
    // A preview changes nothing; the request may also come as JSON text.
    let before = p_rev(&mut p);
    let mut host = ProjectHost { project: &mut p };
    let mut app = AppCtx::assistant(&mut host);
    let r =
        call(&mut app, "edit_timeline", json!({"request": "{\"op\":\"delete\",\"items\":[\"b\"]}", "preview": true}));
    assert_eq!(r.json["dryRun"], true);
    assert_eq!(p_rev(&mut p), before);
    assert_eq!(p.latest_record().unwrap().unwrap().source, Source::Agent);
}

fn p_rev(p: &mut Project) -> i64 {
    p.rev()
}
