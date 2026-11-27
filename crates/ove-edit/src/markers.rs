//! Markers in ripples (add-markers-keyframe-lanes-and-versions D4): when a request asks markers to follow, they move
//! through the ripple's time map like items do. A point marker inside a removed span is removed; a range marker that
//! overlaps a removed span shrinks, and one that lies inside it is removed. Markers are timeline-wide, so they follow
//! the time map of the edit's main track: the track of the edited item, or the first target track.

use serde_json::Map;

use ove_core::model::{Document, Marker};
use ove_core::ops::{Op, apply_batch};
use ove_core::time::{Micros, micros_to_json};
use ove_core::timemap::{Step, TimeMap};

use crate::{Plan, Request};

/// The track whose time map markers follow for a request.
pub fn main_track(doc: &Document, req: &Request) -> Option<String> {
    let item_track = |id: &String| doc.item(id).map(|i| i.track_id.clone());
    match req {
        Request::RippleTrim { item, .. } => item_track(item),
        Request::RippleDelete { items, tracks, .. } => {
            tracks.first().cloned().or_else(|| items.first().and_then(item_track))
        }
        Request::Insert { track, .. } | Request::CloseGap { track, .. } => Some(track.clone()),
        Request::Extract { tracks, .. } => tracks.first().cloned(),
        Request::Split { items, tracks, .. } => items.first().and_then(item_track).or_else(|| tracks.first().cloned()),
        Request::Trim { item, .. }
        | Request::Slip { item, .. }
        | Request::Slide { item, .. }
        | Request::Move { item, .. } => item_track(item),
        Request::Roll { left, .. } => item_track(left),
        Request::Delete { items, .. } => items.first().and_then(item_track),
        Request::Overwrite { track, .. } => Some(track.clone()),
        Request::Lift { tracks, .. } => tracks.first().cloned(),
    }
}

/// The steps markers follow: the main track's, else the first track that moved.
fn steps<'a>(map: &'a TimeMap, track: Option<&str>) -> &'a [Step] {
    track
        .and_then(|t| map.tracks.get(t))
        .filter(|s| !s.is_empty())
        .or_else(|| map.tracks.values().find(|s| !s.is_empty()))
        .map_or(&[], Vec::as_slice)
}

/// Where a marker goes: `None` when it is removed, else its new start and duration.
pub fn map_marker(steps: &[Step], at: Micros, dur: Micros) -> Option<(Micros, Micros)> {
    let point = dur == 0;
    let (mut s, mut e) = (at, at + dur);
    for st in steps {
        if st.delta < 0 {
            let (gone_from, gone_to) = (st.at + st.delta, st.at);
            let inside = if point { s > gone_from && s < gone_to } else { s >= gone_from && e <= gone_to };
            if inside {
                return None;
            }
        }
        let ns = st.map(s);
        e = if point { ns } else { st.map_end(e).max(ns) };
        s = ns;
    }
    Some((s, e - s))
}

/// The ops that move the markers of `doc` through `map` (following `track`'s steps).
pub fn marker_ops(doc: &Document, map: &TimeMap, track: Option<&str>) -> Vec<Op> {
    let steps = steps(map, track);
    if steps.is_empty() {
        return Vec::new();
    }
    let mut ops = Vec::new();
    let mut removed = Vec::new();
    for m in doc.markers_in_order() {
        match map_marker(steps, m.at, m.dur) {
            None => removed.push(m.id.clone()),
            Some((at, dur)) if (at, dur) != (m.at, m.dur) => ops.push(update(m, at, dur)),
            Some(_) => {}
        }
    }
    if !removed.is_empty() {
        ops.push(Op::RemoveMarkers { ids: removed });
    }
    ops
}

fn update(m: &Marker, at: Micros, dur: Micros) -> Op {
    let mut set = Map::new();
    if at != m.at {
        set.insert("at".into(), micros_to_json(at));
    }
    if dur != m.dur {
        set.insert("dur".into(), micros_to_json(dur));
    }
    Op::UpdateMarker { id: m.id.clone(), set, unset: Vec::<String>::new() }
}

/// Makes a planned ripple carry the markers of `before` (the document the plan was made on) with it: the marker ops
/// join the plan's batch, so the edit stays one revision. Plans without a ripple are unchanged.
pub fn follow(before: &Document, req: &Request, plan: &mut Plan) {
    if plan.time_map.is_empty() || before.project.markers.is_empty() {
        return;
    }
    let ops = marker_ops(before, &plan.time_map, main_track(before, req).as_deref());
    if ops.is_empty() {
        return;
    }
    // Marker ops only touch markers, which the planner's ops never do, so they apply on top of its result.
    if let Ok(a) = apply_batch(&plan.result, &ops) {
        plan.result = a.doc;
        plan.ops.extend(ops);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Micros = 1_000_000;

    #[test]
    fn a_removed_span_moves_later_markers_and_removes_points_inside() {
        // Remove [4 s, 6 s).
        let steps = [Step { at: 6 * S, delta: -2 * S }];
        assert_eq!(map_marker(&steps, 3 * S, 0), Some((3 * S, 0)), "before: stays");
        assert_eq!(map_marker(&steps, 4 * S, 0), Some((4 * S, 0)), "at the start of the span: stays");
        assert_eq!(map_marker(&steps, 5 * S, 0), None, "inside: removed");
        assert_eq!(map_marker(&steps, 6 * S, 0), Some((4 * S, 0)), "at the end of the span: moves to its start");
        assert_eq!(map_marker(&steps, 20 * S, 0), Some((18 * S, 0)));
    }

    #[test]
    fn range_markers_shrink_or_go() {
        let steps = [Step { at: 6 * S, delta: -2 * S }];
        assert_eq!(map_marker(&steps, 3 * S, 2 * S), Some((3 * S, S)), "overlaps the start: shrinks");
        assert_eq!(map_marker(&steps, 5 * S, 3 * S), Some((4 * S, 2 * S)), "overlaps the end: shrinks");
        assert_eq!(map_marker(&steps, 3 * S, 5 * S), Some((3 * S, 3 * S)), "spans it: shrinks by its length");
        assert_eq!(map_marker(&steps, 4 * S, S), None, "inside: removed");
    }

    #[test]
    fn an_insert_pushes_later_markers() {
        let steps = [Step { at: 2 * S, delta: S }];
        assert_eq!(map_marker(&steps, S, 0), Some((S, 0)));
        assert_eq!(map_marker(&steps, 2 * S, 0), Some((3 * S, 0)));
        assert_eq!(map_marker(&steps, S, 2 * S), Some((S, 3 * S)), "a range across the insert grows");
    }
}
