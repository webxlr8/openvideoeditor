//! Ripple revisions record their time maps, undo and redo record the inverse and the same map, and
//! `ripples_since` returns them oldest first (add-edit-planners-and-links 3.1).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_core::timemap::{Step, TimeMap};
use serde_json::json;

#[test]
fn ripple_maps_are_recorded_and_survive_reopen() {
    let d = tempfile::tempdir().unwrap();
    let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = lib.create("Ripples").unwrap();
    p.apply(
        vec![
            Op::AddTrack { track: json!({"id": "v", "name": "V", "order": 0, "type": "video"}) },
            Op::AddItem { item: json!({"id": "a", "trackId": "v", "type": "text", "startTime": 0, "endTime": 2}) },
            Op::AddItem { item: json!({"id": "b", "trackId": "v", "type": "text", "startTime": 2, "endTime": 4}) },
        ],
        Source::Gui,
        None,
    )
    .unwrap();
    let base = p.rev();
    let mut map = TimeMap::default();
    map.push("v", Step { at: 2_000_000, delta: -2_000_000 });
    p.apply_mapped(
        vec![
            Op::RemoveItems { ids: vec!["a".into()] },
            Op::UpdateItem { id: "b".into(), set: json!({"startTime": 0, "endTime": 2}).as_object().unwrap().clone() },
        ],
        Source::Gui,
        Some("ripple delete".into()),
        map.clone(),
    )
    .unwrap();
    p.apply(
        vec![Op::UpdateItem { id: "b".into(), set: json!({"name": "B"}).as_object().unwrap().clone() }],
        Source::Gui,
        None,
    )
    .unwrap();
    let r = p.ripples_since(base);
    assert_eq!(r.len(), 1, "plain edits are not ripples");
    assert_eq!(r[0].map, map);
    assert!(r[0].touched.contains("a") && r[0].touched.contains("b"));
    // Undo of the name edit, then of the ripple: the second records the inverse map.
    p.undo().unwrap();
    p.undo().unwrap();
    let r = p.ripples_since(base);
    assert_eq!(r.len(), 2);
    assert_eq!(r[1].map, map.inverse());
    p.redo().unwrap();
    assert_eq!(p.ripples_since(base).last().unwrap().map, map);
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let p =
        ove_store::project::Project::open(&bundle, ove_store::project::Mode::Write, Arc::new(ove_core::SystemClock))
            .unwrap();
    assert_eq!(p.ripples_since(base).len(), 3);
}
