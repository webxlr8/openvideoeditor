//! Markers in the projection (add-markers-keyframe-lanes-and-versions tasks 1.2 and 1.3): print/parse round trip
//! with marker generators, and the keyed-entity merge for markers.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::BTreeSet;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::testgen::{arb_document_with_markers, arb_marker_op};
use ove_projection::{ApplyInput, Prefer, normal, plan, print};
use proptest::prelude::*;
use serde_json::{Map, json};

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

/// Applies the projection printed from `f` (base `b`) onto `c`; returns the plan and the resulting document.
fn apply_file(b: &Document, f: &Document, c: &Document, prefer: Prefer) -> (ove_projection::Plan, Document) {
    let text = print(f, &Default::default());
    let p =
        plan(&ApplyInput { text: &text, current: c, base_for: &|_| Ok(b.clone()), bundle: None, prefer, now_ms: 7 })
            .unwrap_or_else(|r| panic!("refused: {r}\n{text}"));
    let out = if p.ops.is_empty() { c.clone() } else { apply_batch(c, &p.ops).unwrap().doc };
    (p, out)
}

fn without_markers(d: &Document) -> Document {
    let ids: Vec<String> = d.project.markers.keys().cloned().collect();
    if ids.is_empty() {
        return d.clone();
    }
    apply_batch(d, &[Op::RemoveMarkers { ids }]).unwrap().doc
}

fn edited(d: &Document, ops: &[Op]) -> Document {
    apply_batch(d, ops).map(|a| a.doc).unwrap_or_else(|_| d.clone())
}

/// Markers of a document compared in normal form.
fn markers(d: &Document) -> Document {
    let mut n = normal::document(d);
    n.project.items = Default::default();
    n.project.tracks.clear();
    n.project.media.clear();
    n.registry.clear();
    n.project.extra.clear();
    n
}

fn arb_disjoint() -> impl Strategy<Value = (Document, Vec<Op>, Vec<Op>)> {
    arb_document_with_markers().prop_flat_map(|b| {
        let f = proptest::collection::vec(arb_marker_op(&b), 1..=4);
        let c = proptest::collection::vec(arb_marker_op(&b), 0..=4);
        (Just(b), f, c)
    })
}

/// The marker ids an op batch touches.
fn marker_ids(ops: &[Op]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for op in ops {
        match op {
            Op::AddMarker { marker } => {
                out.insert(marker["id"].as_str().unwrap_or_default().to_string());
            }
            Op::UpdateMarker { id, .. } => {
                out.insert(id.clone());
            }
            Op::RemoveMarkers { ids } => out.extend(ids.iter().cloned()),
            _ => {}
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// Round trip: a document's markers, printed, parsed and converted back onto the same document without
    /// markers, equal the originals.
    #[test]
    fn markers_round_trip(d in arb_document_with_markers()) {
        let bare = without_markers(&d);
        let (_, out) = apply_file(&bare, &d, &bare, Prefer::File);
        prop_assert_eq!(markers(&out), markers(&d));
        // And the projection applies onto its own document as a no-op.
        let (p, _) = apply_file(&d, &d, &d, Prefer::File);
        prop_assert!(p.ops.is_empty(), "{:?}", p.ops);
    }

    /// Concurrent edits to different markers never conflict, and both sides' edits are kept.
    #[test]
    fn different_markers_never_conflict((b, fops, cops) in arb_disjoint()) {
        let touched_f = marker_ids(&fops);
        let cops: Vec<Op> = cops.into_iter().filter(|o| marker_ids(std::slice::from_ref(o)).is_disjoint(&touched_f)).collect();
        let f = edited(&b, &fops);
        let c = edited(&b, &cops);
        let (p, out) = apply_file(&b, &f, &c, Prefer::Editor);
        prop_assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
        for id in b.project.markers.keys().chain(f.project.markers.keys()).chain(c.project.markers.keys()) {
            let want = if touched_f.contains(id) { f.marker(id) } else { c.marker(id) };
            let got = out.marker(id);
            prop_assert_eq!(
                got.map(normal::marker_view),
                want.map(normal::marker_view),
                "marker {}", id
            );
        }
    }
}

fn base() -> Document {
    let d = Document::new("p", "Markers", 1);
    apply_batch(
        &d,
        &[
            Op::AddMarker { marker: json!({"id": "m1", "at": 1, "color": "red"}) },
            Op::AddMarker { marker: json!({"id": "m2", "at": 4, "assignee": "agent"}) },
        ],
    )
    .unwrap()
    .doc
}

fn update(id: &str, k: &str, v: serde_json::Value) -> Op {
    let mut set = Map::new();
    set.insert(k.into(), v);
    Op::UpdateMarker { id: id.into(), set, unset: vec![] }
}

#[test]
fn user_and_agent_edit_different_markers() {
    let b = base();
    let user = edited(&b, &[update("m1", "color", json!("blue"))]);
    let agent_file = edited(&b, &[update("m2", "status", json!("review"))]);
    let (p, out) = apply_file(&b, &agent_file, &user, Prefer::File);
    assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
    assert_eq!(out.marker("m1").unwrap().str_field("color"), Some("blue"));
    assert_eq!(out.marker("m2").unwrap().status, "review");
    assert_eq!(p.summary(), "changed 1 marker");
}

#[test]
fn different_fields_of_one_marker_merge_and_the_same_field_conflicts() {
    let b = base();
    let user = edited(&b, &[update("m1", "color", json!("blue"))]);
    let file = edited(&b, &[update("m1", "note", json!("tighten"))]);
    let (p, out) = apply_file(&b, &file, &user, Prefer::File);
    assert!(p.conflicts.is_empty());
    let m = out.marker("m1").unwrap();
    assert_eq!((m.str_field("color"), m.str_field("note")), (Some("blue"), Some("tighten")));

    let file = edited(&b, &[update("m1", "color", json!("green"))]);
    let (p, out) = apply_file(&b, &file, &user, Prefer::Editor);
    assert_eq!(p.conflicts.len(), 1);
    assert_eq!((p.conflicts[0].id.as_str(), p.conflicts[0].field.as_str()), ("m1", "color"));
    assert_eq!(out.marker("m1").unwrap().str_field("color"), Some("blue"));
}

#[test]
fn a_new_marker_in_the_file_gets_an_id_and_applying_twice_adds_it_once() {
    let b = base();
    let mut text = print(&b, &Default::default());
    text = text.replace(
        "  <timeline aspect=\"16:9\">\n",
        "  <timeline aspect=\"16:9\">\n    <marker at={9} name=\"New\" />\n",
    );
    let input = |cur: &Document| {
        plan(&ApplyInput {
            text: &text,
            current: cur,
            base_for: &|_| Ok(b.clone()),
            bundle: None,
            prefer: Prefer::File,
            now_ms: 3,
        })
        .unwrap()
    };
    let p = input(&b);
    let once = apply_batch(&b, &p.ops).unwrap().doc;
    assert_eq!(once.project.markers.len(), 3);
    let added = once.project.markers.values().find(|m| m.str_field("name") == Some("New")).unwrap();
    assert!(added.id.starts_with("mk_"), "{}", added.id);
    assert!(input(&once).ops.is_empty(), "the same file applied twice adds nothing");
}
