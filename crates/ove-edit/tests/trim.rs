//! Trim with source sync, ported from the desktop app's `trimmed` helper tests when trim moved onto the planner
//! (task 2.6). A 25 fps timeline, so the shortest item is one 40 ms frame as before.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

use serde_json::json;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::time::Micros;
use ove_edit::{Edge, Request, plan};

const S: Micros = 1_000_000;

fn doc(media_secs: Option<f64>, speed: f64, reverse: bool) -> Document {
    let mut data = json!({"type": "video", "fileName": "c.mov"});
    if let Some(d) = media_secs {
        data["duration"] = json!(d);
    }
    let mut item = json!({"id": "c", "trackId": "t", "type": "video", "startTime": 10, "endTime": 20, "trimStart": 2,
        "mediaItemId": "m", "playbackSpeed": speed});
    if reverse {
        item["playbackDirection"] = json!("reverse");
    }
    apply_batch(
        &Document::new("p", "Trim", 1),
        &[
            Op::UpdateProject { set: json!({"fps": 25}).as_object().unwrap().clone(), unset: vec![] },
            Op::AddTrack { track: json!({"id": "t", "name": "V", "order": 0, "type": "video"}) },
            Op::AddMedia { media: json!({"id": "m", "data": data}), entry: None },
            Op::AddItem { item },
        ],
    )
    .unwrap()
    .doc
}

/// `(start, end, trimStart)` after trimming `edge` to `to`.
fn trimmed(d: &Document, edge: Edge, to: Micros) -> (Micros, Micros, Micros) {
    let p = plan(d, &Request::Trim { item: "c".into(), edge, to, linked: true, crossfade: false }).unwrap();
    let it = p.result.item("c").unwrap();
    (it.start, it.end, it.time_field("trimStart"))
}

#[test]
fn trim_start_moves_the_in_point() {
    let c = doc(Some(30.0), 1.0, false);
    assert_eq!(trimmed(&c, Edge::Start, 12 * S), (12 * S, 20 * S, 4 * S));
    // Cannot go before the first source frame: in-point 2 s → earliest start 8 s.
    assert_eq!(trimmed(&c, Edge::Start, 5 * S), (8 * S, 20 * S, 0));
    // At 2× speed, 1 s of timeline is 2 s of source.
    let fast = doc(None, 2.0, false);
    assert_eq!(trimmed(&fast, Edge::Start, 11 * S), (11 * S, 20 * S, 4 * S));
}

#[test]
fn trim_end_stays_within_the_source() {
    let c = doc(Some(15.0), 1.0, false);
    // Source is 15 s, in-point 2 s → at most 13 s of clip.
    assert_eq!(trimmed(&c, Edge::End, 40 * S), (10 * S, 23 * S, 2 * S));
    assert_eq!(trimmed(&c, Edge::End, 9 * S).1, 10 * S + 40_000);
    let r: Result<Request, _> = serde_json::from_value(json!({"op": "trim", "item": "c", "edge": "middle", "to": 0}));
    assert!(r.is_err());
}

#[test]
fn reverse_trim_moves_the_other_source_edge() {
    let c = doc(Some(30.0), 1.0, true);
    // The start edge shows the latest source frame: trimming it leaves the in-point alone.
    assert_eq!(trimmed(&c, Edge::Start, 12 * S), (12 * S, 20 * S, 2 * S));
    // Extending the start is limited by the source length: 30 s source, in-point 2 s → at most 28 s of clip.
    assert_eq!(trimmed(&c, Edge::Start, 0), (0, 20 * S, 2 * S));
    assert_eq!(trimmed(&doc(Some(14.0), 1.0, true), Edge::Start, 0), (8 * S, 20 * S, 2 * S));
    // The end edge shows the in-point: pulling it in by 3 s moves the in-point 3 s later.
    let open = doc(None, 1.0, true);
    assert_eq!(trimmed(&open, Edge::End, 17 * S), (10 * S, 17 * S, 5 * S));
    // Extending the end reveals earlier source, down to the first frame (in-point 2 s → end at most 22 s).
    assert_eq!(trimmed(&open, Edge::End, 40 * S), (10 * S, 22 * S, 0));
}
