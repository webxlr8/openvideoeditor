//! Actors on revisions and actor-scoped undo: agents undo only their own changes, a person's edits survive, and
//! undoing over a later change to the same field is refused.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::{Actor, ActorKind, Source};
use ove_store::project::{Mode, Project};
use ove_tools::{BundleCtx, Fail, Receipt, Surface};
use serde_json::{Map, Value, json};

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "t_main", "type": "text", "startTime": 0, "endTime": 1, "name": "A", "textContent": "a"}},
        {"op": "addItem", "item": {"id": "b", "trackId": "t_main", "type": "text", "startTime": 1, "endTime": 2, "name": "B", "textContent": "b"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

/// An agent session over MCP.
fn agent(lib: &Path, session: &str, client: &str) -> BundleCtx {
    let mut c = BundleCtx::open(Some(lib), Surface::Mcp, Source::Mcp).unwrap();
    c.actor = Some(Actor {
        kind: ActorKind::Mcp,
        session: Some(session.into()),
        turn: Some("t1".into()),
        client: Some(client.into()),
    });
    c
}

fn call(ctx: &mut BundleCtx, name: &str, a: Value) -> Result<Receipt, Fail> {
    ove_tools::call(ctx, name, a.as_object().unwrap())
}

/// An agent sets one field of an item by a text edit.
fn agent_set(ctx: &mut BundleCtx, id: &str, attr: &str, from: &str, to: &str) -> i64 {
    let r = call(ctx, "print_timeline", json!({"project": "Demo"})).unwrap();
    let text = r.json["text"].as_str().unwrap();
    let line = ove_tools::edits::elements(text).remove(id).unwrap();
    let edits = json!([{"oldText": line, "newText": line.replace(&format!("{attr}={from}"), &format!("{attr}={to}"))}]);
    let r = call(ctx, "apply_timeline", json!({"project": "Demo", "rev": r.json["rev"], "edits": edits})).unwrap();
    assert_eq!(r.json["changed"], true, "{}", r.json);
    r.json["rev"].as_i64().unwrap()
}

fn person(bundle: &Path, id: &str, set: Value) -> i64 {
    let mut p = Project::open(bundle, Mode::Write, clock()).unwrap();
    let set: Map<String, Value> = set.as_object().unwrap().clone();
    let r = p.apply(vec![Op::UpdateItem { id: id.into(), set }], Source::Gui, None).unwrap().record.rev;
    p.close().unwrap();
    r
}

fn item(bundle: &Path, id: &str, field: &str) -> Option<Value> {
    Project::open(bundle, Mode::ReadOnly, clock()).unwrap().doc().item(id).unwrap().get(field)
}

#[test]
fn revisions_record_their_actor_and_old_records_have_none() {
    let (_d, lib, bundle) = fixture();
    // The fixture's revision was written without an actor, as every release before this one wrote them.
    let revs = bundle.join(".revs");
    let old: Vec<String> = std::fs::read_dir(&revs)
        .unwrap()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().contains("snap"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    assert!(old.iter().all(|t| !t.contains("\"actor\"")));
    let mut one = agent(&lib, "s1", "claude-code");
    let mut two = agent(&lib, "s2", "codex");
    agent_set(&mut one, "a", "name", "\"A\"", "\"One\"");
    agent_set(&mut two, "b", "name", "\"B\"", "\"Two\"");
    let h = Project::open(&bundle, Mode::ReadOnly, clock()).unwrap().history().unwrap();
    assert_eq!(h[0].actor, None, "the old revision opens with no actor");
    let last: Vec<&Actor> = h.iter().rev().take(2).map(|e| e.actor.as_ref().unwrap()).collect();
    assert_eq!(last[0].session.as_deref(), Some("s2"));
    assert_eq!(last[0].client.as_deref(), Some("codex"));
    assert_eq!(last[1].session.as_deref(), Some("s1"));
    assert_eq!(last[1].client.as_deref(), Some("claude-code"));
    // The command line's history shows them.
    let text = call(&mut one, "timeline_history", json!({"project": "Demo"})).unwrap().text;
    assert!(text.contains("mcp (codex)") && text.contains("mcp (claude-code)"), "{text}");
}

#[test]
fn undo_reverts_only_the_agents_change_and_the_persons_edits_survive() {
    let (_d, lib, bundle) = fixture();
    let mut ag = agent(&lib, "s1", "claude-code");
    person(&bundle, "a", json!({"name": "Person 1"}));
    agent_set(&mut ag, "b", "textContent", "\"b\"", "\"agent\"");
    person(&bundle, "a", json!({"textContent": "Person 2"}));
    let r = call(&mut ag, "undo_timeline", json!({"project": "Demo"})).unwrap();
    assert_eq!(r.json["changed"], true);
    assert_eq!(item(&bundle, "b", "textContent"), Some(json!("b")), "the agent's change is reverted");
    assert_eq!(item(&bundle, "a", "name"), Some(json!("Person 1")));
    assert_eq!(item(&bundle, "a", "textContent"), Some(json!("Person 2")));
    // Again: nothing of its own is left, and the person's changes are never undone.
    assert_eq!(call(&mut ag, "undo_timeline", json!({"project": "Demo"})).unwrap_err().code, "undo.nothing");
    assert_eq!(item(&bundle, "a", "name"), Some(json!("Person 1")));
}

#[test]
fn undoing_over_a_later_change_to_the_same_field_is_refused() {
    let (_d, lib, bundle) = fixture();
    let mut ag = agent(&lib, "s1", "claude-code");
    agent_set(&mut ag, "a", "start", "{0}", "{0.5}");
    person(&bundle, "a", json!({"startTime": 0.75}));
    let e = call(&mut ag, "undo_timeline", json!({"project": "Demo"})).unwrap_err();
    assert_eq!(e.code, "undo.conflict");
    assert_eq!(e.details["conflicts"][0]["item"], "a", "{}", e.details);
    assert!(e.message.contains("a startTime"), "{}", e.message);
    assert_eq!(Project::open(&bundle, Mode::ReadOnly, clock()).unwrap().doc().item("a").unwrap().start, 750_000);
}

#[test]
fn another_actors_revision_cannot_be_undone_by_an_agent() {
    let (_d, lib, bundle) = fixture();
    let mut one = agent(&lib, "s1", "claude-code");
    let mut two = agent(&lib, "s2", "codex");
    let theirs = agent_set(&mut two, "b", "name", "\"B\"", "\"Two\"");
    let persons = person(&bundle, "a", json!({"name": "P"}));
    for rev in [theirs, persons] {
        let e = call(&mut one, "undo_timeline", json!({"project": "Demo", "rev": rev})).unwrap_err();
        assert_eq!(e.code, "undo.notYours");
    }
    assert_eq!(call(&mut one, "undo_my_changes", json!({"project": "Demo"})).unwrap_err().code, "undo.nothing");
    assert_eq!(
        call(&mut one, "undo_my_changes", json!({"project": "Demo", "scope": "any"})).unwrap_err().code,
        "undo.scope"
    );
    assert_eq!(item(&bundle, "b", "name"), Some(json!("Two")));
}

#[test]
fn undo_my_changes_reverts_a_session_or_one_turn_as_one_revision() {
    let (_d, lib, bundle) = fixture();
    let mut ag = agent(&lib, "s1", "claude-code");
    agent_set(&mut ag, "a", "name", "\"A\"", "\"A1\"");
    ag.actor.as_mut().unwrap().turn = Some("t2".into());
    agent_set(&mut ag, "b", "name", "\"B\"", "\"B2\"");
    person(&bundle, "a", json!({"textContent": "kept"}));
    // One turn.
    let r = call(&mut ag, "undo_my_changes", json!({"project": "Demo", "scope": "turn"})).unwrap();
    assert_eq!(r.json["undoneRevs"].as_array().unwrap().len(), 1);
    assert_eq!(item(&bundle, "b", "name"), Some(json!("B")));
    assert_eq!(item(&bundle, "a", "name"), Some(json!("A1")));
    // The rest of the session.
    let r = call(&mut ag, "undo_my_changes", json!({"project": "Demo"})).unwrap();
    assert_eq!(r.json["undoneRevs"].as_array().unwrap().len(), 1, "the undone turn is not undone twice");
    assert_eq!(item(&bundle, "a", "name"), Some(json!("A")));
    assert_eq!(item(&bundle, "a", "textContent"), Some(json!("kept")));
    // The command line may undo any session's changes, by name.
    let mut other = agent(&lib, "s9", "x");
    agent_set(&mut other, "b", "name", "\"B\"", "\"S9\"");
    let mut cli = BundleCtx::open(Some(&lib), Surface::Cli, Source::Cli).unwrap();
    let r = ove_tools::call(
        &mut cli,
        "undo_my_changes",
        json!({"project": "Demo", "scope": "any", "session": "s9"}).as_object().unwrap(),
    );
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(item(&bundle, "b", "name"), Some(json!("B")));
}
