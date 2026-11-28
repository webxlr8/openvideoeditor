//! The edit contract: stale-tolerant text edits rebased through the 3-way merge, and receipts whose printed lines
//! anchor the next edit.
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
use ove_tools::{BundleCtx, Fail, Receipt, Surface};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

/// A library with one project, "Demo": `n` one-second titles on Main, `i0` … `i{n-1}`.
fn fixture(n: usize) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Demo").unwrap();
    let mut ops: Vec<Op> = vec![
        serde_json::from_value(
            json!({"op": "addTrack", "track": {"id": "t_text", "name": "Text", "order": 0, "type": "video"}}),
        )
        .unwrap(),
        serde_json::from_value(
            json!({"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 1, "type": "video"}}),
        )
        .unwrap(),
    ];
    for i in 0..n {
        ops.push(
            serde_json::from_value(
                json!({"op": "addItem", "item": {"id": format!("i{i}"), "trackId": "t_main", "type": "text",
                "startTime": i, "endTime": i + 1, "textContent": format!("T{i}"), "name": format!("Item {i}")}}),
            )
            .unwrap(),
        );
    }
    p.apply(ops, Source::Gui, None).unwrap();
    p.flush_projection();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn mcp(lib: &Path) -> BundleCtx {
    BundleCtx::open(Some(lib), Surface::Mcp, Source::Mcp).unwrap()
}

fn call(ctx: &mut BundleCtx, name: &str, a: Value) -> Result<Receipt, Fail> {
    ove_tools::call(ctx, name, a.as_object().unwrap())
}

/// The printed text and rev an agent reads.
fn print(ctx: &mut BundleCtx) -> (String, i64) {
    let r = call(ctx, "print_timeline", json!({"project": "Demo"})).unwrap();
    (r.json["text"].as_str().unwrap().to_string(), r.json["rev"].as_i64().unwrap())
}

fn line_of(text: &str, id: &str) -> String {
    ove_tools::edits::elements(text).remove(id).unwrap_or_else(|| panic!("no element {id}"))
}

/// A person's edit in the editor, as its own revision.
fn gui(bundle: &Path, id: &str, set: Value) {
    let mut p = Project::open(bundle, Mode::Write, clock()).unwrap();
    let set: Map<String, Value> = set.as_object().unwrap().clone();
    p.apply(vec![Op::UpdateItem { id: id.into(), set }], Source::Gui, None).unwrap();
    p.close().unwrap();
}

fn doc(bundle: &Path) -> ove_core::model::Document {
    Project::open(bundle, Mode::ReadOnly, clock()).unwrap().doc().clone()
}

fn apply(ctx: &mut BundleCtx, rev: i64, edits: Value, prefer: Option<&str>) -> Result<Receipt, Fail> {
    let mut a = json!({"project": "Demo", "rev": rev, "edits": edits});
    if let Some(p) = prefer {
        a["prefer"] = json!(p);
    }
    call(ctx, "apply_timeline", a)
}

#[test]
fn an_edit_against_an_older_rev_keeps_the_persons_concurrent_edit() {
    let (_d, lib, bundle) = fixture(3);
    let mut ctx = mcp(&lib);
    let (text, rev) = print(&mut ctx);
    // The person moves i0 after the agent printed.
    gui(&bundle, "i0", json!({"startTime": 10, "endTime": 11}));
    let old = line_of(&text, "i1");
    let r = apply(&mut ctx, rev, json!([{"oldText": old, "newText": old.replace("T1", "Agent")}]), None).unwrap();
    assert_eq!(r.json["conflicts"], json!([]), "{}", r.json);
    let d = doc(&bundle);
    assert_eq!(d.item("i0").unwrap().start, 10_000_000, "the person's move is kept");
    assert_eq!(d.item("i1").unwrap().get("textContent"), Some(json!("Agent")));
    assert!(r.json["rev"].as_i64().unwrap() > rev);
}

#[test]
fn a_real_conflict_with_the_editor_preferred_keeps_the_persons_value_and_lists_it() {
    let (_d, lib, bundle) = fixture(3);
    let mut ctx = mcp(&lib);
    let (text, rev) = print(&mut ctx);
    gui(&bundle, "i2", json!({"textContent": "Person"}));
    let old = line_of(&text, "i2");
    let r =
        apply(&mut ctx, rev, json!([{"oldText": old, "newText": old.replace("T2", "Agent")}]), Some("editor")).unwrap();
    assert_eq!(doc(&bundle).item("i2").unwrap().get("textContent"), Some(json!("Person")));
    let c = &r.json["conflicts"][0];
    assert_eq!(c["id"], "i2", "{}", r.json);
    assert_eq!(c["field"], "textContent");
}

#[test]
fn an_anchor_that_is_not_in_the_printed_rev_is_refused_naming_the_edit() {
    let (_d, lib, bundle) = fixture(2);
    let mut ctx = mcp(&lib);
    let (text, rev) = print(&mut ctx);
    let ok = line_of(&text, "i0");
    let before = doc(&bundle);
    let e = apply(
        &mut ctx,
        rev,
        json!([{"oldText": ok, "newText": ok.replace("T0", "x")}, {"oldText": "<text id=\"nope\"", "newText": ""}]),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, "edit.anchor");
    assert_eq!(e.details["index"], 1);
    assert!(e.message.contains("edit 1"), "{}", e.message);
    assert_eq!(doc(&bundle), before, "nothing was applied");
    // A revision that no longer exists.
    let e = apply(&mut ctx, 5, json!([{"oldText": ok, "newText": ok}]), None).unwrap_err();
    assert_eq!(e.code, "rev.pruned");
    assert!(e.message.contains("Print it again"));
}

#[test]
fn a_new_item_comes_back_with_its_id_and_a_line_that_anchors_the_next_edit() {
    let (_d, lib, _bundle) = fixture(1);
    let mut ctx = mcp(&lib);
    let (text, rev) = print(&mut ctx);
    let anchor = text.lines().find(|l| l.contains("<track id=\"t_text\"")).unwrap().to_string();
    let added = format!("{anchor}\n      <text start={{0}} end={{2}} textContent=\"Hello\" />");
    let r = apply(&mut ctx, rev, json!([{"oldText": anchor, "newText": added}]), None).unwrap();
    let item = &r.json["changedItems"][0];
    let id = item["id"].as_str().unwrap().to_string();
    let line = item["text"].as_str().unwrap().to_string();
    assert!(line.contains(&format!("id=\"{id}\"")) && line.contains("Hello"), "{line}");
    assert_eq!(r.json["added"], 1);
    // The next edit uses the returned line as its anchor, against the returned rev, without printing again.
    let next = r.json["rev"].as_i64().unwrap();
    let r2 = apply(&mut ctx, next, json!([{"oldText": line, "newText": line.replace("Hello", "Hi")}]), None).unwrap();
    assert_eq!(r2.json["updated"], 1, "{}", r2.json);
}

#[test]
fn a_shift_of_twelve_items_is_one_rule() {
    let (_d, lib, _bundle) = fixture(12);
    let mut ctx = mcp(&lib);
    let (text, rev) = print(&mut ctx);
    let mut edits = Vec::new();
    for i in 0..12 {
        let l = line_of(&text, &format!("i{i}"));
        let moved =
            l.replace(&format!("start={{{i}}} end={{{}}}", i + 1), &format!("start={{{}}} end={{{}}}", i + 2, i + 3));
        assert_ne!(l, moved);
        edits.push(json!({"oldText": l, "newText": moved}));
    }
    let r = apply(&mut ctx, rev, Value::Array(edits), None).unwrap();
    assert_eq!(r.json["shifted"], json!([{"track": "t_main", "from": 0.0, "by": 2.0, "count": 12}]), "{}", r.json);
    assert_eq!(r.json["changedItems"], json!([]));
    assert_eq!(r.json["trackOrderChanged"], false);
}

/// Receipts for a fixed edit, without values that depend on the clock.
#[test]
fn golden_receipt() {
    let (_d, lib, _bundle) = fixture(4);
    let mut ctx = mcp(&lib);
    let (text, rev) = print(&mut ctx);
    let a = line_of(&text, "i0");
    let b = line_of(&text, "i1");
    let r = apply(
        &mut ctx,
        rev,
        json!([
            {"oldText": format!("{a}\n"), "newText": ""},
            {"oldText": b, "newText": b.replace("T1", "Renamed")},
        ]),
        None,
    )
    .unwrap();
    let mut v = r.json.clone();
    for k in ["rev", "baseRev"] {
        v.as_object_mut().unwrap().remove(k);
    }
    let want = json!({
        "added": 0, "updated": 1, "removed": 1,
        "removedItems": [{"id": "i0", "name": "Item 0"}],
        "conflicts": [], "warnings": [],
        "changedItems": [{"id": "i1", "text": "      <text id=\"i1\" name=\"Item 1\" start={1} end={2} textContent=\"Renamed\" />"}],
        "moreChanged": 0, "shifted": [], "trackOrderChanged": false,
        "changed": true, "viaApp": false, "summary": "updated 1 item, removed 1 item (i0 \"Item 0\")",
    });
    assert_eq!(v, want, "{}", serde_json::to_string_pretty(&v).unwrap());
}

#[test]
fn every_returned_line_anchors_the_next_edit() {
    let (_d, lib, _bundle) = fixture(40);
    let mut ctx = mcp(&lib);
    let (text, mut rev) = print(&mut ctx);
    let mut lines: Vec<String> = (0..40).map(|i| line_of(&text, &format!("i{i}"))).collect();
    // 25 edits in a row, each anchored only on lines the previous receipts returned.
    for step in 0..25 {
        let n = (step * 7) % 40;
        let old = lines[n].clone();
        let new = old.replace("textContent=\"", &format!("textContent=\"{step}-"));
        let r = apply(&mut ctx, rev, json!([{"oldText": old, "newText": new}]), None).unwrap();
        rev = r.json["rev"].as_i64().unwrap();
        for c in r.json["changedItems"].as_array().unwrap() {
            let i: usize = c["id"].as_str().unwrap()[1..].parse().unwrap();
            lines[i] = c["text"].as_str().unwrap().to_string();
        }
        assert_eq!(lines[n], new, "the receipt returns the line as printed");
    }
}

/// What an agent's change to one item does, for the rebase oracle.
#[derive(Debug, Clone)]
enum GuiOp {
    Move(usize, i64),
    Rename(usize),
    Retext(usize),
}

/// Cases for the rebase property (2,000 by default; `OVE_PROPTEST_CASES`, the knob all property tests share, changes
/// it; `OVE_PROP_CASES` is still read as a fallback).
fn cases() -> u32 {
    ["OVE_PROPTEST_CASES", "OVE_PROP_CASES"]
        .iter()
        .find_map(|k| std::env::var(k).ok().and_then(|v| v.parse().ok()))
        .unwrap_or(2000)
}

/// One project held open in this process, as the app holds it: the person's edits and the agent's calls go to it.
fn held(n: usize) -> (tempfile::TempDir, Project) {
    let (d, _lib, bundle) = fixture(n);
    let p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    (d, p)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// Edits against rev N−k, with up to 5 person edits in between, give the same document as rebasing the agent's
    /// changes by hand onto the current document (the agent's value wins a same-field conflict, `prefer: file`).
    #[test]
    fn stale_edits_match_a_manual_rebase(
        n in 3usize..8,
        gui_ops in proptest::collection::vec((0usize..3, 0usize..8, 1i64..20), 0..5),
        agent in proptest::collection::btree_set(0usize..8, 1..3),
    ) {
        let (_d, mut p) = held(n);
        let call = |p: &mut Project, name: &str, a: Value| {
            let mut host = ove_tools::ProjectHost { project: p };
            let mut ctx = ove_tools::AppCtx { host: &mut host, source: Source::Mcp, surface: Surface::Mcp, actor: None };
            ove_tools::call(&mut ctx, name, a.as_object().unwrap())
        };
        let printed = call(&mut p, "print_timeline", json!({})).unwrap();
        let text = printed.json["text"].as_str().unwrap().to_string();
        let rev = printed.json["rev"].as_i64().unwrap();
        let ops: Vec<GuiOp> = gui_ops.iter().map(|(k, i, x)| match k {
            0 => GuiOp::Move(i % n, *x),
            1 => GuiOp::Rename(i % n),
            _ => GuiOp::Retext(i % n),
        }).collect();
        for (step, op) in ops.iter().enumerate() {
            let (id, set) = match op {
                GuiOp::Move(i, x) => (i, json!({"startTime": 20 + x * 2 + step as i64 * 50, "endTime": 21 + x * 2 + step as i64 * 50})),
                GuiOp::Rename(i) => (i, json!({"name": format!("Person {step}")})),
                GuiOp::Retext(i) => (i, json!({"textContent": format!("P{step}")})),
            };
            let set: Map<String, Value> = set.as_object().unwrap().clone();
            p.apply(vec![Op::UpdateItem { id: format!("i{id}"), set }], Source::Gui, None).unwrap();
        }
        let agent: Vec<usize> = agent.into_iter().map(|i| i % n).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let edits: Vec<Value> = agent.iter().map(|i| {
            let l = line_of(&text, &format!("i{i}"));
            json!({"oldText": l, "newText": l.replace(&format!("textContent=\"T{i}\""), &format!("textContent=\"A{i}\""))})
        }).collect();
        let current = p.doc().clone();
        call(&mut p, "apply_timeline", json!({"rev": rev, "edits": edits})).unwrap();
        // The manual rebase: the agent's values set on the current document.
        let mut want = current.clone();
        for i in &agent {
            let set: Map<String, Value> = json!({"textContent": format!("A{i}")}).as_object().unwrap().clone();
            want = ove_core::apply_batch(&want, &[Op::UpdateItem { id: format!("i{i}"), set }]).unwrap().doc;
        }
        prop_assert_eq!(&p.doc().project.items, &want.project.items);
        prop_assert_eq!(&p.doc().project.tracks, &want.project.tracks);
    }
}
