//! One test per spec scenario the planners carry (specs/edit-planners, specs/clip-links).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

mod common;

use serde_json::json;

use common::*;
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_edit::retime::property_at;
use ove_edit::{Edge, Range, Request, plan};

fn clip(gap: i64, len: i64, linked: bool) -> Gen {
    Gen { gap, len, trim: 0, fast: false, reverse: false, keyframes: None, linked }
}

/// v0 [0,2 s) v1 [2,4 s) v2 [4,6 s) on v1 (linked to a0..a2 on a1), music [0,8 s) and a title [5,6 s).
fn base(music_sync: bool, title_locked: bool) -> Document {
    build(
        &[clip(0, 50, true), clip(0, 50, true), clip(0, 50, true)],
        &[(0, 200)],
        &[(125, 25)],
        &Tracks { music_sync, title_locked },
    )
}

fn with(doc: &Document, ops: Vec<Op>) -> Document {
    apply_batch(doc, &ops).unwrap().doc
}

#[test]
fn snapped_split() {
    let d = base(true, false);
    let p =
        plan(&d, &Request::Split { items: vec!["v0".into()], tracks: vec![], at: 1_010_000, linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v0").unwrap().end, S, "1.01 s snaps to 1.00 s at 25 fps");
    // The linked audio was split at the same point, and the right halves share a new link.
    assert_eq!(after.item("a0").unwrap().end, S);
    let rights: Vec<_> = after.project.items.values().filter(|i| i.start == S).collect();
    assert_eq!(rights.len(), 2);
    assert_eq!(rights[0].fields.get("link"), rights[1].fields.get("link"));
    assert_ne!(rights[0].fields.get("link"), after.item("v0").unwrap().fields.get("link"));
}

#[test]
fn ripple_delete_closes_the_gap() {
    let d = base(true, false);
    let p = plan(&d, &Request::RippleDelete { items: vec!["v1".into()], tracks: vec![], ranges: vec![], linked: true })
        .unwrap();
    let after = apply_checked(&d, &p);
    assert!(after.item("v1").is_none() && after.item("a1").is_none(), "the linked audio goes too");
    assert_eq!(after.item("v2").unwrap().start, 2 * S);
    assert_eq!(after.item("a2").unwrap().start, 2 * S);
    // The title on a sync-locked track moves 2 s earlier; the music (sync-locked, spanning the cut) loses the same
    // 2 s of its source: it ends at 2 s and a new piece plays its source from 4 s on.
    assert_eq!(after.item("t0").unwrap().start, 3 * S);
    assert_eq!(after.item("mu0").unwrap().end, 2 * S);
    let rest: Vec<_> = after.items_on_track("music").into_iter().filter(|i| i.id != "mu0").collect();
    assert_eq!((rest[0].start, rest[0].end, rest[0].time_field("trimStart")), (2 * S, 6 * S, 4 * S));
}

#[test]
fn unlocked_track_stays() {
    let d = base(false, false);
    let p = plan(&d, &Request::RippleDelete { items: vec!["v1".into()], tracks: vec![], ranges: vec![], linked: true })
        .unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("mu0").unwrap().start, 0);
    assert_eq!(after.item("mu0").unwrap().end, 8 * S);
}

#[test]
fn blocked_ripple() {
    let d = base(true, true);
    let r = plan(&d, &Request::RippleDelete { items: vec!["v1".into()], tracks: vec![], ranges: vec![], linked: true })
        .unwrap_err();
    assert_eq!(r.code, "ripple.blocked");
    assert_eq!(r.culprits, vec!["t0".to_string()]);
}

#[test]
fn preview_does_not_change_the_document() {
    let d = base(true, false);
    let before = d.clone();
    let _ = plan(&d, &Request::RippleDelete { items: vec!["v1".into()], tracks: vec![], ranges: vec![], linked: true })
        .unwrap();
    assert_eq!(d, before);
}

#[test]
fn keyframe_survives_a_split() {
    let d = build(&[clip(0, 500, false)], &[], &[], &Tracks { music_sync: true, title_locked: false });
    let d = with(
        &d,
        vec![Op::UpdateItem {
            id: "v0".into(),
            set: json!({"keyframeTracks": [{"property": "opacity", "keyframes": [{"time": 0, "value": 0}, {"time": 10, "value": 1}, {"time": 20, "value": 0}]}]})
                .as_object()
                .unwrap()
                .clone(),
        }],
    );
    let was = property_at(d.item("v0").unwrap(), "opacity", 10 * S).unwrap();
    let p = plan(&d, &Request::Split { items: vec!["v0".into()], tracks: vec![], at: 8 * S, linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    let right = after.project.items.values().find(|i| i.start == 8 * S).unwrap();
    assert_eq!(property_at(right, "opacity", 10 * S).unwrap(), was);
}

#[test]
fn lift_and_extract_a_range() {
    let d = base(false, false);
    let range = Range { start: 4 * S, end: 6 * S };
    let p = plan(&d, &Request::Lift { tracks: vec!["v1".into()], range }).unwrap();
    let after = apply_checked(&d, &p);
    assert!(after.item("v2").is_none());
    assert!(p.time_map.is_empty());
    let d = with(
        &base(false, false),
        vec![Op::UpdateItem { id: "v2".into(), set: json!({"endTime": 8}).as_object().unwrap().clone() }],
    );
    let p = plan(&d, &Request::Extract { tracks: vec!["v1".into()], range }).unwrap();
    let after = apply_checked(&d, &p);
    let v2 = after.item("v2").unwrap();
    assert_eq!((v2.start, v2.end), (4 * S, 6 * S));
    assert_eq!(v2.time_field("trimStart"), 2 * S, "the part after the range is what remains");
}

#[test]
fn slip_beyond_the_source_stops_at_the_limit() {
    let d = base(true, false);
    // The media is 60 s long; v0 uses [0, 2 s). A 100 s slip stops at 58 s.
    let p = plan(&d, &Request::Slip { item: "v0".into(), delta: 100 * S, linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v0").unwrap().time_field("trimStart"), 58 * S);
    assert_eq!(after.item("a0").unwrap().time_field("trimStart"), 58 * S);
    assert!(p.limited.contains(&"v0".to_string()));
}

#[test]
fn move_a_linked_pair_and_one_side_only() {
    let d = base(true, false);
    let p = plan(&d, &Request::Move { item: "v2".into(), start: 5 * S, track: None, linked: true, crossfade: false })
        .unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v2").unwrap().start, 5 * S);
    assert_eq!(after.item("a2").unwrap().start, 5 * S);
    let p = plan(&d, &Request::Move { item: "a2".into(), start: 5 * S, track: None, linked: false, crossfade: false })
        .unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v2").unwrap().start, 4 * S);
    assert_eq!(after.item("a2").unwrap().start, 5 * S);
}

#[test]
fn l_cut_trims_only_the_audio() {
    let d = base(true, false);
    let p = plan(&d, &Request::Trim { item: "a2".into(), edge: Edge::End, to: 7 * S, linked: false, crossfade: false })
        .unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("a2").unwrap().end, 7 * S);
    assert_eq!(after.item("v2").unwrap().end, 6 * S);
    assert_eq!(after.item("a2").unwrap().fields.get("link"), after.item("v2").unwrap().fields.get("link"));
}

#[test]
fn delete_a_linked_pair() {
    let d = base(true, false);
    let p = plan(&d, &Request::Delete { items: vec!["v1".into()], linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    assert!(after.item("a1").is_none());
    assert_eq!(after.item("v2").unwrap().start, 4 * S, "a plain delete leaves the gap");
}

#[test]
fn a_new_overlap_is_refused_unless_a_crossfade_is_asked() {
    let d = base(true, false);
    let req = |crossfade| Request::Trim { item: "v0".into(), edge: Edge::End, to: 3 * S, linked: false, crossfade };
    let r = plan(&d, &req(false)).unwrap_err();
    assert_eq!(r.code, "overlap.created");
    assert_eq!(r.culprits, vec!["v0".to_string(), "v1".to_string()]);
    assert!(plan(&d, &req(true)).is_ok());
}

#[test]
fn ripple_trim_roll_and_close_gap() {
    let d = base(true, false);
    // Ripple trim v0's end 1 s earlier: everything after moves 1 s earlier.
    let p = plan(&d, &Request::RippleTrim { item: "v0".into(), edge: Edge::End, to: S, linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v0").unwrap().end, S);
    assert_eq!(after.item("v1").unwrap().start, S);
    assert_eq!(after.item("a1").unwrap().start, S);
    // Ripple trim v1's start 1 s later: v1 keeps its start, loses its head, the rest follows.
    let p = plan(&d, &Request::RippleTrim { item: "v1".into(), edge: Edge::Start, to: 3 * S, linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    let v1 = after.item("v1").unwrap();
    assert_eq!((v1.start, v1.end, v1.time_field("trimStart")), (2 * S, 3 * S, S));
    assert_eq!(after.item("v2").unwrap().start, 3 * S);
    // Roll the v1|v2 edit point to 5 s.
    let p = plan(&d, &Request::Roll { left: "v1".into(), right: "v2".into(), to: 5 * S, linked: true }).unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v1").unwrap().end, 5 * S);
    assert_eq!(after.item("v2").unwrap().start, 5 * S);
    assert_eq!(after.item("a1").unwrap().end, 5 * S, "the linked audio edit point rolls too");
    // Close a gap: delete v1 (leaving a gap), then close it.
    let gap = apply_checked(&d, &plan(&d, &Request::Delete { items: vec!["v1".into()], linked: true }).unwrap());
    let r = plan(&gap, &Request::CloseGap { track: "v1".into(), at: 3 * S }).unwrap_err();
    assert_eq!(r.code, "ripple.blocked", "the music under the gap would be cut");
    let no_music = with(&gap, vec![Op::RemoveItems { ids: vec!["mu0".into()] }]);
    let p = plan(&no_music, &Request::CloseGap { track: "v1".into(), at: 3 * S }).unwrap();
    let after = apply_checked(&no_music, &p);
    assert_eq!(after.item("v2").unwrap().start, 2 * S);
}

#[test]
fn insert_and_overwrite() {
    let d = base(false, false);
    let new = vec![json!({"type": "video", "startTime": 0, "endTime": 1, "mediaItemId": "m"})];
    let p = plan(&d, &Request::Insert { track: "v1".into(), at: 3 * S, items: new.clone() }).unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.items_on_track("v1").len(), 5, "v1 is cut at 3 s and one item added");
    assert_eq!(after.item("v2").unwrap().start, 5 * S);
    let p = plan(&d, &Request::Overwrite { track: "v1".into(), at: 3 * S, items: new }).unwrap();
    let after = apply_checked(&d, &p);
    assert_eq!(after.item("v1").unwrap().end, 3 * S);
    assert_eq!(after.item("v2").unwrap().start, 4 * S);
    assert_eq!(after.items_on_track("v1").len(), 4);
}

#[test]
fn requests_parse_from_json_with_seconds() {
    let r: Request =
        serde_json::from_value(json!({"op": "rippleDelete", "tracks": ["a1"], "ranges": [{"start": 12.3, "end": 14}]}))
            .unwrap();
    assert_eq!(
        r,
        Request::RippleDelete {
            items: vec![],
            tracks: vec!["a1".into()],
            ranges: vec![Range { start: 12_300_000, end: 14 * S }],
            linked: true
        }
    );
    assert!(serde_json::from_value::<Request>(json!({"op": "split", "at": 1, "bogus": 1})).is_err());
}
