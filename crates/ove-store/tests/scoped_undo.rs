//! Undo scoped to an actor (add-markers-keyframe-lanes-and-versions task 4.4). Property, over random sequences of
//! edits by the user and two agent sessions: undoing one agent's changes either reverts exactly that agent's
//! revisions (the result equals replaying everyone else's edits alone) or, when a later revision by someone else
//! changed the same field, is refused with `undo.conflict` naming it and changes nothing.
//!
//! Each case commits durable revisions, so the default case count is lower than the pure property suites
//! (`OVE_PROPTEST_CASES` overrides it).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod common;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::revision::{Actor, ActorKind, Source};
use ove_store::project::Project;
use ove_store::undo::{UndoError, UndoScope};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(500)
}

fn agent(session: &str) -> Actor {
    Actor { kind: ActorKind::Mcp, session: Some(session.into()), turn: None, client: None }
}

fn user() -> Actor {
    Actor::new(ActorKind::User)
}

const ITEMS: usize = 4;
const FIELDS: &[&str] = &["name", "opacity", "textContent"];

/// One edit: who (0 user, 1 agent A, 2 agent B), which item, which field, a value seed.
type Step = (u8, usize, usize, u8);

fn setup() -> Vec<Op> {
    let mut ops = vec![Op::AddTrack { track: json!({"id": "t", "name": "T", "order": 0, "type": "video"}) }];
    for i in 0..ITEMS {
        ops.push(Op::AddItem {
            item: json!({"id": format!("i{i}"), "trackId": "t", "type": "text", "startTime": i, "endTime": i + 1}),
        });
    }
    ops.push(Op::AddMarker { marker: json!({"id": "m", "at": 1}) });
    ops
}

fn op(step: &Step) -> Op {
    let (_, item, field, v) = *step;
    let value: Value = match FIELDS[field] {
        "opacity" => json!(f64::from(v % 100) / 100.0),
        f => json!(format!("{f} {v}")),
    };
    if item == ITEMS {
        let mut set = Map::new();
        set.insert("note".into(), json!(format!("note {v}")));
        return Op::UpdateMarker { id: "m".into(), set, unset: vec![] };
    }
    let mut set = Map::new();
    set.insert(FIELDS[field].into(), value);
    Op::UpdateItem { id: format!("i{item}"), set }
}

fn actor_of(who: u8) -> Actor {
    match who {
        0 => user(),
        1 => agent("A"),
        _ => agent("B"),
    }
}

fn source_of(who: u8) -> Source {
    if who == 0 { Source::Gui } else { Source::Mcp }
}

/// The (object, field) a step touches.
fn target(s: &Step) -> (usize, usize) {
    if s.1 == ITEMS { (ITEMS, 0) } else { (s.1, s.2) }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn undo_by_actor_never_reverts_another_actors_revision(
        steps in proptest::collection::vec((0u8..3, 0usize..=ITEMS, 0usize..FIELDS.len(), any::<u8>()), 1..8)
    ) {
        let d = tempfile::tempdir().unwrap();
        let mut p = Project::create(&d.path().join("p.ove"), "p", "P", common::clock()).unwrap();
        p.apply(setup(), Source::Gui, None).unwrap();
        let start: Document = p.doc().clone();
        for s in &steps {
            p.apply_as(vec![op(s)], source_of(s.0), None, Some(actor_of(s.0))).unwrap();
        }
        let before = p.doc().clone();
        let a_steps: Vec<usize> = steps.iter().enumerate().filter(|(_, s)| s.0 == 1).map(|(i, _)| i).collect();
        // A conflict: someone else changed a field A changed, after A did.
        let conflict = a_steps.iter().any(|&i| steps[i + 1..].iter().any(|l| l.0 != 1 && target(l) == target(&steps[i])));
        // Everyone else's edits alone, in order.
        let mut others = start.clone();
        for s in steps.iter().filter(|s| s.0 != 1) {
            others = apply_batch(&others, &[op(s)]).unwrap().doc;
        }
        match p.undo_scope(&UndoScope::Session(agent("A")), "undo agent A".into(), Some(agent("A"))) {
            Ok(c) => {
                prop_assert!(p.doc().content_eq(&others), "only agent A's revisions were reverted");
                prop_assert_eq!(c.record.source, Source::Undo);
            }
            // Nothing of A's still changes anything: the document already equals everyone else's edits.
            Err(UndoError::Nothing) => prop_assert!(others.content_eq(&before)),
            Err(UndoError::Conflict(c)) => {
                prop_assert!(conflict, "refused without a conflict: {:?}", c);
                prop_assert!(!c.is_empty());
                prop_assert!(p.doc().content_eq(&before), "a refused undo changes nothing");
            }
            Err(UndoError::Store(e)) => prop_assert!(false, "{e}"),
        }
    }
}

#[test]
fn the_agent_undoes_only_its_edit_of_b_and_a_later_user_edit_refuses() {
    // Spec scenarios "Agent undoes its own edit" and "Conflict refuses undo".
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::create(&d.path().join("p.ove"), "p", "P", common::clock()).unwrap();
    p.apply(setup(), Source::Gui, None).unwrap();
    let edit = |p: &mut Project, item: &str, name: &str, who: Actor| {
        let mut set = Map::new();
        set.insert("name".into(), json!(name));
        let src = if who.kind == ActorKind::User { Source::Gui } else { Source::Mcp };
        p.apply_as(vec![Op::UpdateItem { id: item.into(), set }], src, None, Some(who)).unwrap();
    };
    edit(&mut p, "i0", "A by user", user());
    edit(&mut p, "i1", "B by agent", agent("A"));
    edit(&mut p, "i2", "C by user", user());
    p.undo_scope(&UndoScope::Session(agent("A")), "undo".into(), None).unwrap();
    let doc = p.doc();
    assert_eq!(doc.item("i0").unwrap().name(), "A by user");
    assert_eq!(doc.item("i1").unwrap().name(), "");
    assert_eq!(doc.item("i2").unwrap().name(), "C by user");

    edit(&mut p, "i1", "B by agent again", agent("A"));
    edit(&mut p, "i1", "B by user", user());
    match p.undo_scope(&UndoScope::Session(agent("A")), "undo".into(), None) {
        Err(UndoError::Conflict(c)) => assert!(c.iter().any(|x| x.item.as_deref() == Some("i1")), "{c:?}"),
        other => panic!("expected a conflict, got {other:?}"),
    }
    // The user's own scope reverts the user's revisions only.
    let users = p.revisions_in_scope(&UndoScope::User).unwrap();
    assert!(
        users.iter().all(|r| r.actor.as_ref().is_some_and(|a| a.kind == ActorKind::User) || r.source == Source::Gui)
    );
}
