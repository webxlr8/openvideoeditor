//! Model round trip, migrations, op rules, invariants, revisions and undo/redo.

use ove_core::invariants::check_document;
use ove_core::migrate::{Opened, open_json};
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::revision::{Clock, Session, Source};
use ove_core::schema::Severity;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicI64, Ordering};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn open(text: &str) -> Document {
    match open_json(text).unwrap() {
        Opened::Current(d) => d,
        other => panic!("unexpected {other:?}"),
    }
}

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
fn clock(t: i64) -> TestClock {
    TestClock(AtomicI64::new(t))
}

/// Two video tracks, one media item, two clips.
fn base() -> Document {
    let mut d = Document::new("p1", "Test", 1000);
    let ops = vec![
        Op::AddTrack { track: json!({"id": "t1", "name": "Main", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "t2", "name": "Music", "order": 1, "type": "audio"}) },
        Op::AddMedia {
            media: json!({"id": "m1", "data": {"type": "video", "fileName": "a.mov", "duration": 10}}),
            entry: Some(json!({"mediaId": "m1", "nativePath": "/a.mov", "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "a", "trackId": "t1", "type": "video", "startTime": 0, "endTime": 2, "mediaItemId": "m1"}),
        },
        Op::AddItem {
            item: json!({"id": "b", "trackId": "t1", "type": "video", "startTime": 2, "endTime": 5, "mediaItemId": "m1"}),
        },
    ];
    d = apply_batch(&d, &ops).unwrap().doc;
    d
}

fn set(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn reject(doc: &Document, ops: Vec<Op>) -> ove_core::BatchError {
    apply_batch(doc, &ops).expect_err("batch should be rejected")
}

// ------------------------------------------------------------------------------------------------ model

#[test]
fn fixtures_round_trip_byte_identical() {
    for name in ["all-types.json", "unknown-fields.json"] {
        let text = fixture(name);
        let doc = open(&text);
        assert_eq!(doc.to_string_pretty(), text, "{name}");
    }
}

#[test]
fn fixture_covers_every_item_type() {
    let doc = open(&fixture("all-types.json"));
    for t in ove_core::schema::ITEM_TYPES {
        assert!(doc.project.items.values().any(|i| i.kind == *t), "missing {t}");
    }
}

#[test]
fn unknown_fields_survive_an_edit() {
    let doc = open(&fixture("unknown-fields.json"));
    let after = apply_batch(&doc, &[Op::UpdateItem { id: "i1".into(), set: set(json!({"opacity": 0.5})) }]).unwrap();
    let text = after.doc.to_string_pretty();
    for needle in
        ["zFuture", "zUnknown", "aUnknown", "scael", "zebra", "checksum", "zTopLevel", "aTopLevel", "unknownProbeField"]
    {
        assert!(text.contains(needle), "{needle} lost");
    }
    // Unknown keys inside closed known objects warn, but do not fail.
    assert!(after.warnings.iter().any(|w| w.code == "object.key.unknown" && w.path.ends_with("transform.scael")));
}

#[test]
fn times_are_microsecond_precision() {
    let doc = base();
    let d = apply_batch(
        &doc,
        &[Op::UpdateItem { id: "a".into(), set: set(json!({"endTime": 1.0000004, "trimStart": 0.1 })) }],
    )
    .unwrap()
    .doc;
    let a = d.item("a").unwrap();
    assert_eq!(a.end, 1_000_000);
    assert_eq!(a.fields["trimStart"], json!(0.1));
    // Repeated float arithmetic cannot drift: 0.1 + 0.2 stored as 0.3.
    let d =
        apply_batch(&d, &[Op::UpdateItem { id: "a".into(), set: set(json!({"trimStart": 0.1 + 0.2})) }]).unwrap().doc;
    assert_eq!(d.item("a").unwrap().fields["trimStart"].to_string(), "0.3");
}

#[test]
fn invalid_document_reports_first_invalid_field_path() {
    let mut v: Value = serde_json::from_str(&fixture("all-types.json")).unwrap();
    v["project"]["timelineItems"][1]["startTime"] = json!("soon");
    let e = open_json(&v.to_string()).unwrap_err();
    assert_eq!(e.path, "$.project.timelineItems[1].startTime");

    let mut v: Value = serde_json::from_str(&fixture("all-types.json")).unwrap();
    v["project"]["timelineItems"][0]["opacity"] = json!(7);
    let e = open_json(&v.to_string()).unwrap_err();
    assert_eq!(e.code, "value.range");
    assert_eq!(e.path, "items[i_text].opacity");

    let e = open_json("{\"version\": 1, \"format\": ").unwrap_err();
    assert_eq!(e.code, "json.syntax");
}

#[test]
fn migration_v0_to_v1() {
    let v0 = json!({
        "version": 0,
        "project": {"id": "old", "name": "Old", "stateUpdatedAt": 5, "aspectRatio": "1:1",
            "trackList": [{"id": "t", "name": "T", "order": 0, "type": "video"}],
            "items": [{"id": "i", "trackId": "t", "type": "text", "startTime": 0, "endTime": 1, "textContent": "x"}],
            "media": []},
        "media": []
    });
    match open_json(&v0.to_string()).unwrap() {
        Opened::Migrated { doc, from_version } => {
            assert_eq!(from_version, 0);
            assert_eq!(doc.version, 1);
            assert_eq!(doc.project.items.len(), 1);
            assert_eq!(doc.project.tracks.len(), 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn newer_version_opens_read_only() {
    let mut v: Value = serde_json::from_str(&fixture("all-types.json")).unwrap();
    v["version"] = json!(99);
    match open_json(&v.to_string()).unwrap() {
        Opened::Newer { version, doc } => {
            assert_eq!(version, 99);
            assert!(doc.is_some());
        }
        other => panic!("{other:?}"),
    }
}

// ------------------------------------------------------------------------------------------------ ops

#[test]
fn add_track_inserts_and_renumbers() {
    let d = apply_batch(
        &base(),
        &[Op::AddTrack { track: json!({"id": "t0", "name": "Top", "order": 0, "type": "video"}) }],
    )
    .unwrap();
    let ids: Vec<_> = d.doc.project.tracks.iter().map(|t| (t.id.as_str(), t.order)).collect();
    assert_eq!(ids, vec![("t0", 0), ("t1", 1), ("t2", 2)]);
    assert_eq!(
        reject(&base(), vec![Op::AddTrack { track: json!({"id": "t1", "name": "x", "order": 0, "type": "video"}) }])
            .code,
        "track.id.unique"
    );
    assert_eq!(reject(&base(), vec![Op::AddTrack { track: json!({"id": "tx", "order": 0}) }]).code, "field.required");
    assert_eq!(
        reject(&base(), vec![Op::AddTrack { track: json!({"id": "tx", "order": 0, "type": "bogus"}) }]).code,
        "enum.known"
    );
}

#[test]
fn update_and_unset_track() {
    let d =
        apply_batch(&base(), &[Op::UpdateTrack { id: "t1".into(), set: set(json!({"name": "A", "locked": true})) }])
            .unwrap()
            .doc;
    assert_eq!(d.track("t1").unwrap().name, "A");
    let d2 = apply_batch(&d, &[Op::UnsetTrackFields { id: "t1".into(), fields: vec!["locked".into()] }]).unwrap().doc;
    assert!(!d2.track("t1").unwrap().fields.contains_key("locked"));
    assert_eq!(
        reject(&base(), vec![Op::UpdateTrack { id: "nope".into(), set: set(json!({"name": "x"})) }]).code,
        "op.target.missing"
    );
    assert_eq!(
        reject(&base(), vec![Op::UpdateTrack { id: "t1".into(), set: set(json!({"order": 3})) }]).code,
        "op.field.immutable"
    );
    assert_eq!(
        reject(&base(), vec![Op::UpdateTrack { id: "t1".into(), set: set(json!({"locked": "yes"})) }]).code,
        "value.type"
    );
    assert_eq!(
        reject(&base(), vec![Op::UnsetTrackFields { id: "t1".into(), fields: vec!["type".into()] }]).code,
        "op.field.required"
    );
}

#[test]
fn remove_and_reorder_tracks() {
    // Removing a track with items is rejected...
    let e = reject(&base(), vec![Op::RemoveTrack { id: "t1".into() }]);
    assert_eq!(e.code, "item.track.exists");
    assert_eq!(e.op_index, Some(0));
    // ...unless the same batch removes or moves them.
    apply_batch(&base(), &[Op::RemoveTrack { id: "t1".into() }, Op::RemoveItems { ids: vec!["a".into(), "b".into()] }])
        .unwrap();
    let moved = apply_batch(
        &base(),
        &[
            Op::RemoveTrack { id: "t1".into() },
            Op::UpdateItem { id: "a".into(), set: set(json!({"trackId": "t2"})) },
            Op::UpdateItem { id: "b".into(), set: set(json!({"trackId": "t2"})) },
        ],
    )
    .unwrap();
    assert_eq!(moved.doc.project.tracks[0].id, "t2");
    assert_eq!(moved.doc.project.tracks[0].order, 0);

    let r = apply_batch(&base(), &[Op::ReorderTracks { order: vec!["t2".into(), "t1".into()] }]).unwrap();
    assert_eq!(r.doc.project.tracks[0].id, "t2");
    assert_eq!(reject(&base(), vec![Op::ReorderTracks { order: vec!["t2".into()] }]).code, "op.reorder.permutation");
    assert_eq!(reject(&base(), vec![Op::RemoveTrack { id: "zz".into() }]).code, "op.target.missing");
}

#[test]
fn item_ops_success_and_rejections() {
    let b = base();
    let d = apply_batch(
        &b,
        &[Op::AddItem {
            item: json!({"id": "c", "trackId": "t1", "type": "text", "startTime": 5, "endTime": 6, "textContent": "x"}),
        }],
    )
    .unwrap()
    .doc;
    assert!(d.item("c").is_some());
    assert_eq!(
        reject(
            &b,
            vec![Op::AddItem {
                item: json!({"id": "a", "trackId": "t1", "type": "video", "startTime": 0, "endTime": 1})
            }]
        )
        .code,
        "item.id.unique"
    );
    assert_eq!(
        reject(&b, vec![Op::AddItem { item: json!({"id": "c", "trackId": "t1", "type": "video", "startTime": 0}) }])
            .code,
        "field.required"
    );
    assert_eq!(
        reject(&b, vec![Op::UpdateItem { id: "a".into(), set: set(json!({"id": "z"})) }]).code,
        "op.field.immutable"
    );
    assert_eq!(
        reject(&b, vec![Op::UpdateItem { id: "q".into(), set: set(json!({"opacity": 1})) }]).code,
        "op.target.missing"
    );
    assert_eq!(
        reject(&b, vec![Op::UpdateItem { id: "a".into(), set: set(json!({"startTime": "x"})) }]).code,
        "value.type"
    );
    assert_eq!(reject(&b, vec![Op::RemoveItems { ids: vec!["a".into(), "q".into()] }]).code, "op.target.missing");
    assert_eq!(
        reject(&b, vec![Op::UnsetItemFields { id: "a".into(), fields: vec!["endTime".into()] }]).code,
        "op.field.required"
    );
    let d = apply_batch(
        &b,
        &[Op::UnsetItemFields { id: "a".into(), fields: vec!["mediaItemId".into(), "notThere".into()] }],
    )
    .unwrap()
    .doc;
    assert!(d.item("a").unwrap().media_id().is_none());
    let d = apply_batch(&b, &[Op::RemoveItems { ids: vec!["a".into()] }]).unwrap().doc;
    assert!(d.item("a").is_none());
}

#[test]
fn aspect_media_and_project_ops() {
    let b = base();
    let d = apply_batch(&b, &[Op::SetAspectRatio { aspect_ratio: "9:16".into() }]).unwrap().doc;
    assert_eq!(d.project.aspect_ratio, "9:16");
    assert_eq!(reject(&b, vec![Op::SetAspectRatio { aspect_ratio: "2:1".into() }]).code, "enum.known");
    let d = apply_batch(&b, &[Op::UpdateProject { set: set(json!({"name": "Renamed", "cast": []})), unset: vec![] }])
        .unwrap()
        .doc;
    assert_eq!(d.project.name, "Renamed");
    assert_eq!(
        reject(&b, vec![Op::UpdateProject { set: set(json!({"id": "x"})), unset: vec![] }]).code,
        "op.field.immutable"
    );
    assert_eq!(reject(&b, vec![Op::RemoveMedia { id: "m1".into() }]).code, "item.media.exists");
    assert_eq!(reject(&b, vec![Op::AddMedia { media: json!({"id": "m1"}), entry: None }]).code, "media.id.unique");
    let d = apply_batch(
        &b,
        &[Op::UpdateMedia {
            id: "m1".into(),
            set: Default::default(),
            unset: vec![],
            entry_set: set(json!({"probeStatus": "probed", "size": 10})),
            entry_unset: vec![],
        }],
    )
    .unwrap()
    .doc;
    assert_eq!(d.registry["m1"].probe_status(), "probed");
    assert_eq!(
        reject(
            &b,
            vec![Op::UpdateMedia {
                id: "m1".into(),
                set: Default::default(),
                unset: vec![],
                entry_set: set(json!({"probeStatus": "lost"})),
                entry_unset: vec![]
            }]
        )
        .code,
        "enum.known"
    );
}

// ------------------------------------------------------------------------------------------------ invariants

#[test]
fn one_failing_case_per_invariant_code() {
    let b = base();
    let cases: Vec<(&str, Vec<Op>)> = vec![
        (
            "item.track.exists",
            vec![Op::AddItem {
                item: json!({"id": "x", "trackId": "nope", "type": "video", "startTime": 0, "endTime": 1}),
            }],
        ),
        ("item.media.exists", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"mediaItemId": "ghost"})) }]),
        ("item.time.order", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"endTime": 0})) }]),
        ("value.nonNegative", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"trimStart": -1})) }]),
        ("value.range", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"volume": 500})) }]),
        ("value.type", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"muted": 1})) }]),
        ("enum.known", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"blendMode": "glow"})) }]),
        ("enum.known", vec![Op::UpdateItem { id: "a".into(), set: set(json!({"type": "hologram"})) }]),
        (
            "item.id.unique",
            vec![Op::AddItem {
                item: json!({"id": "b", "trackId": "t1", "type": "video", "startTime": 0, "endTime": 1}),
            }],
        ),
        ("track.id.unique", vec![Op::AddTrack { track: json!({"id": "t2", "order": 0, "type": "audio"}) }]),
        ("media.id.unique", vec![Op::AddMedia { media: json!({"id": "m1"}), entry: None }]),
    ];
    for (code, ops) in cases {
        let e = reject(&b, ops);
        assert_eq!(e.code, code, "{e}");
    }
    // Negative start.
    assert_eq!(
        reject(&b, vec![Op::UpdateItem { id: "a".into(), set: set(json!({"startTime": -1})) }]).code,
        "value.nonNegative"
    );
    // Track order contiguity and registry references can only break in a hand-edited file: checked on open.
    let mut d = b.clone();
    d.project.tracks[1].order = 5;
    assert!(check_document(&d).iter().any(|i| i.code == "track.order.contiguous"));
    let mut d = b.clone();
    d.registry.insert(
        "ghost".into(),
        ove_core::RegistryEntry {
            media_id: "ghost".into(),
            native_path: "/x".into(),
            owned_by_bundle: false,
            fields: Default::default(),
        },
    );
    assert!(check_document(&d).iter().any(|i| i.code == "registry.media.exists"));
    let mut d = b;
    d.format = "other".into();
    assert!(check_document(&d).iter().any(|i| i.code == "project.format"));
}

#[test]
fn atomic_batch_names_op_index() {
    let b = base();
    let ops = vec![
        Op::UpdateItem { id: "a".into(), set: set(json!({"opacity": 0.5})) },
        Op::UpdateItem { id: "b".into(), set: set(json!({"opacity": 0.5})) },
        Op::SetAspectRatio { aspect_ratio: "1:1".into() },
        Op::AddItem { item: json!({"id": "bad", "trackId": "missing", "type": "video", "startTime": 0, "endTime": 1}) },
        Op::UpdateTrack { id: "t1".into(), set: set(json!({"name": "x"})) },
    ];
    let e = apply_batch(&b, &ops).unwrap_err();
    assert_eq!(e.op_index, Some(3));
    assert_eq!(e.code, "item.track.exists");
    // The input document is untouched.
    assert_eq!(b, base());
}

#[test]
fn every_issue_in_fixture_is_valid() {
    let doc = open(&fixture("all-types.json"));
    let errs: Vec<_> = check_document(&doc).into_iter().filter(|i| i.severity == Severity::Error).collect();
    assert!(errs.is_empty(), "{errs:?}");
}

// ------------------------------------------------------------------------------------------------ revisions

#[test]
fn monotonic_rev_within_same_millisecond() {
    let c = clock(5000);
    let mut s = Session::new(base());
    let r1 =
        s.commit(s.prepare(vec![Op::SetAspectRatio { aspect_ratio: "1:1".into() }], Source::Gui, None, &c).unwrap());
    let r2 =
        s.commit(s.prepare(vec![Op::SetAspectRatio { aspect_ratio: "4:5".into() }], Source::Cli, None, &c).unwrap());
    assert!(r2.rev > r1.rev);
    assert_eq!(r2.parent, r1.rev);
    assert_eq!(s.doc().project.rev, r2.rev);
    c.0.store(10, Ordering::SeqCst); // clock moves backwards
    let r3 =
        s.commit(s.prepare(vec![Op::SetAspectRatio { aspect_ratio: "16:9".into() }], Source::Agent, None, &c).unwrap());
    assert_eq!(r3.rev, r2.rev + 1);
    assert_eq!(r3.source, Source::Agent);
    assert_eq!(r3.summary, "1 aspect change");
}

#[test]
fn undo_an_agent_edit_of_40_ops() {
    let c = clock(10_000);
    let start = base();
    let mut s = Session::new(start.clone());
    let mut ops = vec![];
    for i in 0..38 {
        ops.push(Op::AddItem {
            item: json!({"id": format!("n{i}"), "trackId": "t2", "type": "audio", "startTime": i, "endTime": i + 1}),
        });
    }
    ops.push(Op::UpdateItem { id: "a".into(), set: set(json!({"opacity": 0.2})) });
    ops.push(Op::RemoveItems { ids: vec!["b".into()] });
    assert_eq!(ops.len(), 40);
    s.commit(s.prepare(ops, Source::Agent, None, &c).unwrap());
    assert_eq!(s.doc().project.items.len(), 39);
    let rec = s.commit(s.prepare_undo(&c).unwrap().unwrap());
    assert_eq!(rec.source, Source::Undo);
    assert!(s.doc().content_eq(&start));
    assert!(!s.can_undo());
    // Redo, then undo again.
    s.commit(s.prepare_redo(&c).unwrap().unwrap());
    assert_eq!(s.doc().project.items.len(), 39);
    s.commit(s.prepare_undo(&c).unwrap().unwrap());
    assert!(s.doc().content_eq(&start));
}

#[test]
fn new_batch_after_undo_clears_redo() {
    let c = clock(1);
    let mut s = Session::new(base());
    s.commit(s.prepare(vec![Op::SetAspectRatio { aspect_ratio: "1:1".into() }], Source::Gui, None, &c).unwrap());
    s.commit(s.prepare_undo(&c).unwrap().unwrap());
    assert!(s.can_redo());
    s.commit(s.prepare(vec![Op::SetAspectRatio { aspect_ratio: "4:3".into() }], Source::Gui, None, &c).unwrap());
    assert!(!s.can_redo());
    assert!(s.prepare_redo(&c).is_none());
}

#[test]
fn restore_is_a_new_revision() {
    let c = clock(1);
    let mut s = Session::new(base());
    let at_r1 = s.doc().clone();
    s.commit(
        s.prepare(
            vec![Op::RemoveItems { ids: vec!["a".into()] }, Op::SetAspectRatio { aspect_ratio: "9:16".into() }],
            Source::Gui,
            None,
            &c,
        )
        .unwrap(),
    );
    s.commit(
        s.prepare(
            vec![
                Op::UpdateTrack { id: "t2".into(), set: set(json!({"name": "Score"})) },
                Op::RemoveTrack { id: "t2".into() },
            ],
            Source::Gui,
            None,
            &c,
        )
        .unwrap(),
    );
    let rec = s.commit(s.prepare_restore(&at_r1, at_r1.project.rev, &c).unwrap().unwrap());
    assert_eq!(rec.source, Source::Restore);
    assert!(s.doc().content_eq(&at_r1));
    // Restoring to the current content is a no-op.
    assert!(s.prepare_restore(&at_r1, 0, &c).is_none());
}

#[test]
fn empty_batch_is_refused() {
    let s = Session::new(base());
    assert_eq!(s.prepare(vec![], Source::Gui, None, &clock(1)).unwrap_err().code, "batch.empty");
}

#[test]
fn ops_json_wire_format() {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "setAspectRatio", "aspectRatio": "1:1"},
        {"op": "removeItems", "ids": ["a"]},
        {"op": "updateMedia", "id": "m1", "entrySet": {"probeStatus": "probed"}}
    ]))
    .unwrap();
    assert_eq!(ops[0], Op::SetAspectRatio { aspect_ratio: "1:1".into() });
    let back = serde_json::to_value(&ops).unwrap();
    assert_eq!(back[2], json!({"op": "updateMedia", "id": "m1", "entrySet": {"probeStatus": "probed"}}));
    assert!(serde_json::from_value::<Op>(json!({"op": "explode"})).is_err());
}
