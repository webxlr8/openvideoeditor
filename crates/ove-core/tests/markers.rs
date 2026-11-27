//! Marker entities (add-markers-keyframe-lanes-and-versions task 1.1): each op, its inverse, and each invariant
//! refusal.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_core::diff::diff_documents;
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch, summarise};
use serde_json::{Map, Value, json};

fn doc_with(markers: &[Value]) -> Document {
    let ops: Vec<Op> = markers.iter().map(|m| Op::AddMarker { marker: m.clone() }).collect();
    apply_batch(&Document::new("p", "Markers", 1), &ops).unwrap().doc
}

fn set(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn add_marker_defaults_to_an_open_point_and_undoes() {
    let base = Document::new("p", "Markers", 1);
    let a = apply_batch(&base, &[Op::AddMarker { marker: json!({"id": "m1", "at": 12}) }]).unwrap();
    let m = a.doc.marker("m1").unwrap();
    assert_eq!((m.at, m.dur, m.status.as_str()), (12_000_000, 0, "open"));
    let back = apply_batch(&a.doc, &a.inverse).unwrap().doc;
    assert!(back.content_eq(&base));
}

#[test]
fn update_marker_sets_head_and_fields_and_undoes() {
    let d = doc_with(&[json!({"id": "m1", "at": 1, "name": "Cut here"})]);
    let a = apply_batch(
        &d,
        &[Op::UpdateMarker {
            id: "m1".into(),
            set: set(&[("at", json!(2.5)), ("dur", json!(1)), ("status", json!("review")), ("color", json!("red"))]),
            unset: vec!["name".into()],
        }],
    )
    .unwrap();
    let m = a.doc.marker("m1").unwrap();
    assert_eq!((m.at, m.dur, m.status.as_str()), (2_500_000, 1_000_000, "review"));
    assert_eq!(m.str_field("color"), Some("red"));
    assert_eq!(m.str_field("name"), None);
    let back = apply_batch(&a.doc, &a.inverse).unwrap().doc;
    assert!(back.content_eq(&d), "{:?}", back.marker("m1"));
}

#[test]
fn remove_markers_and_undo() {
    let d = doc_with(&[json!({"id": "m1", "at": 1}), json!({"id": "m2", "at": 3, "note": "x"})]);
    let a = apply_batch(&d, &[Op::RemoveMarkers { ids: vec!["m1".into(), "m2".into()] }]).unwrap();
    assert!(a.doc.project.markers.is_empty());
    assert!(apply_batch(&a.doc, &a.inverse).unwrap().doc.content_eq(&d));
}

fn refused(d: &Document, op: Op) -> &'static str {
    apply_batch(d, &[op]).expect_err("refused").code
}

#[test]
fn invariant_refusals() {
    let d = doc_with(&[json!({"id": "m1", "at": 1})]);
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m1", "at": 2}) }), "marker.id.unique");
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m2", "at": -1}) }), "value.nonNegative");
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m2", "at": 1, "dur": -2}) }), "value.nonNegative");
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m2", "at": 1, "status": "done"}) }), "enum.known");
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m2"}) }), "field.required");
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m2", "at": 1, "color": 3}) }), "value.type");
    let long = "n".repeat(4001);
    assert_eq!(refused(&d, Op::AddMarker { marker: json!({"id": "m2", "at": 1, "note": long}) }), "marker.note.length");
    let ok = "n".repeat(4000);
    assert!(apply_batch(&d, &[Op::AddMarker { marker: json!({"id": "m2", "at": 1, "note": ok}) }]).is_ok());
    let upd = |k: &str, v: Value| Op::UpdateMarker { id: "m1".into(), set: set(&[(k, v)]), unset: vec![] };
    assert_eq!(refused(&d, upd("status", json!("done"))), "enum.known");
    assert_eq!(refused(&d, upd("id", json!("m9"))), "op.field.immutable");
    assert_eq!(refused(&d, upd("at", json!("soon"))), "value.type");
    assert_eq!(
        refused(&d, Op::UpdateMarker { id: "m1".into(), set: Map::new(), unset: vec!["at".into()] }),
        "op.field.required"
    );
    assert_eq!(refused(&d, upd("at", json!(-3))), "value.nonNegative");
    assert_eq!(
        refused(&d, Op::UpdateMarker { id: "ghost".into(), set: Map::new(), unset: vec![] }),
        "op.target.missing"
    );
    assert_eq!(refused(&d, Op::RemoveMarkers { ids: vec!["ghost".into()] }), "op.target.missing");
    // Markers are not project metadata.
    assert_eq!(
        refused(&d, Op::UpdateProject { set: set(&[("markers", json!([]))]), unset: vec![] }),
        "op.field.immutable"
    );
}

#[test]
fn markers_round_trip_through_json_in_time_order() {
    let d = doc_with(&[
        json!({"id": "b", "at": 5, "assignee": "agent", "zUnknown": {"k": 1}}),
        json!({"id": "a", "at": 5, "dur": 2, "status": "resolved"}),
        json!({"id": "c", "at": 1}),
    ]);
    let v = d.to_json();
    let ids: Vec<&str> =
        v["project"]["markers"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["c", "a", "b"]);
    let back = Document::from_json(&v).unwrap();
    assert_eq!(back, d);
    // A document without markers writes no `markers` key.
    assert!(Document::new("p", "x", 1).to_json()["project"].get("markers").is_none());
}

#[test]
fn diff_produces_marker_ops() {
    let a = doc_with(&[json!({"id": "m1", "at": 1, "name": "a"}), json!({"id": "m2", "at": 2})]);
    let b = doc_with(&[json!({"id": "m1", "at": 1.5, "color": "blue"}), json!({"id": "m3", "at": 9})]);
    let ops = diff_documents(&a, &b);
    let got = apply_batch(&a, &ops).unwrap().doc;
    assert!(got.content_eq(&b), "{ops:?}");
    assert_eq!(summarise(&ops), "1 marker removed, 1 marker update, 1 marker added");
}
