//! Markers follow ripples (add-markers-keyframe-lanes-and-versions task 2.1): a ripple never moves a marker that sits
//! before the removed range, markers after it shift by its length, and nothing moves when markers do not follow.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

mod common;

use proptest::prelude::*;
use serde_json::json;

use common::*;
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_edit::markers::follow;
use ove_edit::{Range, Request, plan};

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

/// `doc` with markers at the given (frame, length in frames) pairs, ids `m0`, `m1`, ….
fn with_markers(doc: &Document, ms: &[(i64, i64)]) -> Document {
    let ops: Vec<Op> = ms
        .iter()
        .enumerate()
        .map(|(n, (at, len))| Op::AddMarker {
            marker: json!({"id": format!("m{n}"), "at": secs(at * F), "dur": secs(len * F)}),
        })
        .collect();
    apply_batch(doc, &ops).unwrap().doc
}

fn arb_case() -> impl Strategy<Value = (Document, Range, Vec<(i64, i64)>)> {
    (
        arb_doc(),
        0i64..200,
        1i64..50,
        prop::collection::vec((0i64..300, prop_oneof![3 => Just(0i64), 1 => 1i64..40]), 0..6),
    )
        .prop_map(|(d, a, l, ms)| (d, Range { start: a * F, end: (a + l) * F }, ms))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn ripples_carry_markers_only_when_asked((doc, range, ms) in arb_case()) {
        let doc = with_markers(&doc, &ms);
        let req = Request::RippleDelete { items: vec![], tracks: vec!["v1".into()], ranges: vec![range], linked: true };
        let Ok(held) = plan(&doc, &req) else { return Ok(()) };
        // Not following: markers stay exactly where they are.
        let after = apply_checked(&doc, &held);
        prop_assert_eq!(&after.project.markers, &doc.project.markers);

        let mut following = held.clone();
        follow(&doc, &req, &mut following);
        let after = apply_checked(&doc, &following);
        let len = range.end - range.start;
        for m in doc.project.markers.values() {
            let got = after.marker(&m.id);
            if m.end() <= range.start {
                // Before the removed range: never moved.
                prop_assert_eq!(got, Some(m.as_ref()), "{}", m.id.clone());
            } else if m.at >= range.end {
                let g = got.unwrap();
                prop_assert_eq!((g.at, g.dur), (m.at - len, m.dur), "{} shifts by the removed length", m.id.clone());
            } else if m.dur == 0 {
                prop_assert!(m.at == range.start || got.is_none(), "a point inside the range is removed");
            } else if let Some(g) = got {
                prop_assert!(g.at <= m.at && g.dur < m.dur, "{} overlapping the range shrinks", m.id.clone());
            } else {
                prop_assert!(m.at >= range.start && m.end() <= range.end, "only a range inside the range is removed");
            }
        }
    }
}

#[test]
fn a_marker_after_a_two_second_ripple_delete() {
    let base = with_markers(&common::build(&[], &[], &[], &Tracks { music_sync: true, title_locked: false }), &[]);
    let doc = apply_batch(
        &base,
        &[
            Op::AddItem { item: json!({"id": "x", "trackId": "v1", "type": "text", "startTime": 0, "endTime": 30}) },
            Op::AddMarker { marker: json!({"id": "m", "at": 20}) },
        ],
    )
    .unwrap()
    .doc;
    let req = Request::RippleDelete {
        items: vec![],
        tracks: vec!["v1".into()],
        ranges: vec![Range { start: 2 * S, end: 4 * S }],
        linked: true,
    };
    let held = plan(&doc, &req).unwrap();
    assert_eq!(apply_checked(&doc, &held).marker("m").unwrap().at, 20 * S, "markers held in place");
    let mut following = held.clone();
    follow(&doc, &req, &mut following);
    assert_eq!(apply_checked(&doc, &following).marker("m").unwrap().at, 18 * S, "marker follows");
    assert_eq!(following.ops.len(), held.ops.len() + 1, "one batch: the marker op joins the ripple's ops");
}
