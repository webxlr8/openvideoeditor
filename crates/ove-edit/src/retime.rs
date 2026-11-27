//! Retiming (design D2): one helper computes an item cut or extended to a new timeline span, for every planner.
//!
//! Moving an edge of an item changes three kinds of field:
//! - the source mapping (`trimStart`, `trimEnd`) of video and audio, which depends on `playbackSpeed` and
//!   `playbackDirection`;
//! - fields stored relative to the item start: keyframe times and caption segments with their word timings;
//! - fields tied to one edge: fades and transitions.
//!
//! The result shows the same source frames and the same animated values at every timeline time that both the old
//! and the new span cover.

use serde_json::{Map, Value, json};

use ove_core::model::{Document, Item};
use ove_core::retime::Key;
use ove_core::time::{Micros, json_to_micros, micros_to_json};

/// Fields that belong to the start edge of an item.
pub const START_EDGE: &[&str] = &["fadeIn", "audioFadeIn", "transitionIn"];
/// Fields that belong to the end edge of an item.
pub const END_EDGE: &[&str] = &["fadeOut", "audioFadeOut", "transitionOut"];

/// Playback speed factor of an item (at least 0.01).
pub fn speed(item: &Item) -> f64 {
    item.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01)
}

/// True when the item plays its source backwards.
pub fn reversed(item: &Item) -> bool {
    item.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse")
}

/// True for item types that map timeline time to source time.
pub fn has_source(item: &Item) -> bool {
    matches!(item.kind.as_str(), "video" | "audio")
}

/// Timeline time scaled to source time.
pub fn scaled(us: Micros, speed: f64) -> Micros {
    (us as f64 * speed).round() as Micros
}

/// Source time scaled to timeline time.
pub fn unscaled(us: Micros, speed: f64) -> Micros {
    (us as f64 / speed).round() as Micros
}

/// Duration of the item's media, when the item plays media whose duration is known.
pub fn media_duration(doc: &Document, item: &Item) -> Option<Micros> {
    if !has_source(item) {
        return None;
    }
    let m = doc.project.media.get(item.media_id()?)?;
    m.data().and_then(|d| d.get("duration")).and_then(json_to_micros)
}

/// Which edge fields a cut keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edges {
    pub keep_start: bool,
    pub keep_end: bool,
}

impl Edges {
    /// A trim: both edges keep their fades and transitions.
    pub const TRIM: Edges = Edges { keep_start: true, keep_end: true };
    /// The left part of a cut.
    pub const LEFT: Edges = Edges { keep_start: true, keep_end: false };
    /// The right part of a cut.
    pub const RIGHT: Edges = Edges { keep_start: false, keep_end: true };
}

/// The span of source an item would use for timeline span `[a, b)`: `(trimStart, trimEnd)` after the cut (trimEnd
/// only when the item has one). Values may be negative when the span reaches past the source start.
pub fn source_after(item: &Item, a: Micros, b: Micros) -> (Micros, Option<Micros>) {
    let s = speed(item);
    let ts = item.time_field("trimStart");
    let te = item.fields.get("trimEnd").and_then(json_to_micros);
    let (head, tail) = (scaled(a - item.start, s), scaled(item.end - b, s));
    if reversed(item) { (ts + tail, te.map(|te| te + head)) } else { (ts + head, te.map(|te| te + tail)) }
}

/// The item cut (or extended) to timeline span `[a, b)` with `a < b`. Keeps the id. Source fields follow speed and
/// direction (clamped at 0); keyframes and caption segments keep their timeline times; edge fields are kept or
/// dropped as `edges` says.
pub fn cut(item: &Item, a: Micros, b: Micros, edges: Edges) -> Item {
    let mut out = item.clone();
    out.start = a;
    out.end = b;
    if has_source(item) {
        let (ts, te) = source_after(item, a, b);
        if ts != item.time_field("trimStart") || item.fields.contains_key("trimStart") {
            out.fields.insert("trimStart".into(), micros_to_json(ts.max(0)));
        }
        if let Some(te) = te {
            out.fields.insert("trimEnd".into(), micros_to_json(te.max(0)));
        }
    }
    // Item-relative fields move when the start moves, and are cut at a new end edge.
    if a != item.start || (b < item.end && !edges.keep_end) {
        let (from, to) = (a - item.start, b - item.start);
        if let Some(tracks) = item.fields.get("keyframeTracks").and_then(Value::as_array) {
            out.fields.insert("keyframeTracks".into(), Value::Array(window_keyframes(tracks, from, to)));
        }
        if let Some(segs) = item.fields.get("captionSegments").and_then(Value::as_array) {
            out.fields.insert("captionSegments".into(), Value::Array(window_segments(segs, from, to)));
        }
    }
    if !edges.keep_start {
        for k in START_EDGE {
            out.fields.shift_remove(*k);
        }
    }
    if !edges.keep_end {
        for k in END_EDGE {
            out.fields.shift_remove(*k);
        }
    }
    out
}

/// The item with its source moved by `delta` of timeline time (a slip): later source for a positive delta. Timeline
/// span, keyframes and caption segments stay. Returns `None` for items without a source mapping.
pub fn slipped(item: &Item, delta: Micros) -> Option<Item> {
    if !has_source(item) {
        return None;
    }
    let s = speed(item);
    let d = scaled(delta, s);
    let mut out = item.clone();
    let ts = item.time_field("trimStart");
    // Forward items show later source when trimStart grows; reversed items too (both ends of the window move).
    out.fields.insert("trimStart".into(), micros_to_json((ts + d).max(0)));
    if let Some(te) = item.fields.get("trimEnd").and_then(json_to_micros) {
        out.fields.insert("trimEnd".into(), micros_to_json((te - d).max(0)));
    }
    Some(out)
}

/// The item moved by `delta` on the timeline (no other field changes).
pub fn shifted(item: &Item, delta: Micros) -> Item {
    let mut out = item.clone();
    out.start += delta;
    out.end += delta;
    out
}

/// The range of timeline times an edge can move to without going past the item's source: `(earliest, latest)`
/// for `edge` given the media duration. `None` bounds are unlimited.
pub fn edge_limits(item: &Item, duration: Option<Micros>, start_edge: bool) -> (Option<Micros>, Option<Micros>) {
    if !has_source(item) || item.media_id().is_none() {
        return (None, None);
    }
    let s = speed(item);
    let ts = item.time_field("trimStart");
    let used = scaled(item.end - item.start, s);
    // Forward: the start edge shows trimStart, the end edge trimStart + used. Reverse: the other way round.
    let towards_source_start = start_edge != reversed(item);
    if towards_source_start {
        // This edge may reveal up to `trimStart` of earlier source.
        let reach = unscaled(ts, s);
        if start_edge { (Some(item.start - reach), None) } else { (None, Some(item.end + reach)) }
    } else {
        let room = duration.map(|d| unscaled((d - ts - used).max(0), s));
        if start_edge { (room.map(|r| item.start - r), None) } else { (None, room.map(|r| item.end + r)) }
    }
}

/// Slip limits as a `(min, max)` delta of timeline time.
pub fn slip_limits(item: &Item, duration: Option<Micros>) -> (Micros, Option<Micros>) {
    let s = speed(item);
    let ts = item.time_field("trimStart");
    let used = scaled(item.end - item.start, s);
    let min = -unscaled(ts, s);
    let max = duration.map(|d| unscaled((d - ts - used).max(0), s));
    (min, max)
}

// ---------------------------------------------------------------------------------------------------------------
// The wave 0 split helpers, kept with their contract (moved from ove-core).
// ---------------------------------------------------------------------------------------------------------------

/// The result of cutting an item in two at a timeline time.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    /// Fields to set on the left part, which keeps the item's id.
    pub left_set: Map<String, Value>,
    /// Optional fields to remove from the left part.
    pub left_unset: Vec<String>,
    /// The complete right part, a new item.
    pub right: Value,
}

/// Cuts `item` at timeline time `at` (strictly inside the item). The left part keeps the id and the start-edge
/// fields; the right part gets `right_id` and the end-edge fields. Both parts show the same source frames and the
/// same animated values at every timeline time as the uncut item. Returns `None` when `at` is not inside the item.
pub fn split(item: &Item, at: Micros, right_id: &str) -> Option<Split> {
    if at <= item.start || at >= item.end {
        return None;
    }
    let left = cut(item, item.start, at, Edges::LEFT);
    let mut right = cut(item, at, item.end, Edges::RIGHT);
    right.id = right_id.to_string();
    let mut left_set = Map::new();
    left_set.insert("endTime".into(), micros_to_json(at));
    for (k, v) in &left.fields {
        if item.fields.get(k) != Some(v) {
            left_set.insert(k.clone(), v.clone());
        }
    }
    let left_unset = item.fields.keys().filter(|k| !left.fields.contains_key(*k)).cloned().collect();
    Some(Split { left_set, left_unset, right: right.to_json() })
}

/// Fields to set when the start edge of `item` moves to `new_start` and the source mapping is the caller's: shifts
/// keyframes and caption segments so they stay at the same timeline times; keyframes and segments that fall before
/// the new start are cut at it.
pub fn rebase_start(item: &Item, new_start: Micros) -> Map<String, Value> {
    let mut set = Map::new();
    let delta = new_start - item.start;
    if delta == 0 {
        return set;
    }
    let len = item.end - new_start;
    if let Some(tracks) = item.fields.get("keyframeTracks").and_then(Value::as_array) {
        set.insert("keyframeTracks".into(), Value::Array(window_keyframes(tracks, delta, delta + len)));
    }
    if let Some(segs) = item.fields.get("captionSegments").and_then(Value::as_array) {
        set.insert("captionSegments".into(), Value::Array(window_segments(segs, delta, delta + len)));
    }
    set
}

/// Value of a keyframe track at item-relative time `t` with the track's easing, or `None` for an empty track.
pub fn track_value(keyframes: &[(Micros, f64)], easing: &str, t: Micros) -> Option<f64> {
    let keys: Vec<Key> = keyframes.iter().map(|&(time, value)| Key { time, value, easing: None }).collect();
    ove_core::retime::value_at(&keys, easing, t)
}

/// Value of an item's keyframed property at timeline time `t`, or `None` when the property has no keyframes. Each
/// segment uses its start keyframe's easing, else the track's.
pub fn property_at(item: &Item, property: &str, t: Micros) -> Option<f64> {
    let track = ove_core::retime::find_track(item.fields.get("keyframeTracks"), property)?;
    ove_core::retime::track_value(track, t - item.start)
}

/// Keyframe tracks restricted to the item-relative window `[from, to]` and shifted so `from` becomes 0. A keyframe is
/// added at each window edge that falls between keyframes, holding the value there, so the curve is unchanged
/// inside the window for linear and hold segments (eased segments keep their end values; the shape between is
/// re-eased). Kept keyframes keep their own easing; the keyframe added at `from` takes the easing of the segment it
/// cuts.
fn window_keyframes(tracks: &[Value], from: Micros, to: Micros) -> Vec<Value> {
    tracks
        .iter()
        .map(|track| {
            let mut out = track.clone();
            if track.get("keyframes").and_then(Value::as_array).is_none() {
                return out;
            }
            let easing = ove_core::retime::track_easing(track);
            let keys = ove_core::retime::keys(track);
            let mut kept: Vec<Key> = Vec::new();
            if let Some(v) = ove_core::retime::value_at(&keys, easing, from)
                && keys.first().is_some_and(|f| f.time < from)
            {
                let cut = keys.iter().rev().find(|k| k.time < from).and_then(|k| k.easing.clone());
                kept.push(Key { time: from, value: v, easing: cut });
            }
            kept.extend(keys.iter().filter(|k| k.time >= from && k.time <= to).cloned());
            if let Some(v) = ove_core::retime::value_at(&keys, easing, to)
                && keys.last().is_some_and(|l| l.time > to)
            {
                kept.push(Key { time: to, value: v, easing: None });
            }
            kept.dedup_by_key(|k| k.time);
            out["keyframes"] = Value::Array(
                kept.into_iter()
                    .map(|k| {
                        let mut v = json!({"time": micros_to_json(k.time - from), "value": k.value});
                        if let Some(e) = k.easing {
                            v["easing"] = e.into();
                        }
                        v
                    })
                    .collect(),
            );
            out
        })
        .collect()
}

/// Caption segments that overlap the item-relative window `[from, to)`, clipped to it and shifted so `from`
/// becomes 0; word timings outside the window are dropped.
fn window_segments(segs: &[Value], from: Micros, to: Micros) -> Vec<Value> {
    let shift = |v: Option<&Value>| v.and_then(json_to_micros).map(|t| t.clamp(from, to) - from);
    segs.iter()
        .filter_map(|seg| {
            let a = seg.get("startTime").and_then(json_to_micros)?;
            let b = seg.get("endTime").and_then(json_to_micros)?;
            if b <= from || a >= to {
                return None;
            }
            let mut out = seg.clone();
            out["startTime"] = micros_to_json(shift(seg.get("startTime"))?);
            out["endTime"] = micros_to_json(shift(seg.get("endTime"))?);
            if let Some(words) = seg.get("wordTimings").and_then(Value::as_array) {
                let kept: Vec<Value> = words
                    .iter()
                    .filter(|w| {
                        let ws = w.get("start").and_then(json_to_micros).unwrap_or(0);
                        let we = w.get("end").and_then(json_to_micros).unwrap_or(ws);
                        we > from && ws < to
                    })
                    .map(|w| {
                        let mut w = w.clone();
                        for k in ["start", "end"] {
                            if let Some(t) = shift(w.get(k)) {
                                w[k] = micros_to_json(t);
                            }
                        }
                        w
                    })
                    .collect();
                out["wordTimings"] = Value::Array(kept);
            }
            Some(out)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Micros = 1_000_000;

    fn item(v: Value) -> Item {
        Item::from_json(&v, "item").expect("valid item")
    }

    fn t(v: &Value) -> Micros {
        json_to_micros(v).expect("time")
    }

    #[test]
    fn forward_split_maps_source_with_speed() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "video", "startTime": 2, "endTime": 6,
            "trimStart": 10, "trimEnd": 1, "mediaItemId": "m", "playbackSpeed": 2}));
        let s = split(&it, 3 * S, "a2").expect("inside");
        // The right part starts 1 s of timeline (2 s of source) later.
        assert_eq!(t(&s.right["trimStart"]), 12 * S);
        assert_eq!(t(&s.right["trimEnd"]), S);
        assert!(!s.left_set.contains_key("trimStart"));
        assert_eq!(t(&s.left_set["trimEnd"]), S + 6 * S);
        assert_eq!(t(&s.left_set["endTime"]), 3 * S);
        assert_eq!(t(&s.right["startTime"]), 3 * S);
    }

    #[test]
    fn reverse_split_gives_earlier_source_to_the_right() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "video", "startTime": 0, "endTime": 4,
            "trimStart": 10, "mediaItemId": "m", "playbackDirection": "reverse"}));
        let s = split(&it, S, "a2").expect("inside");
        assert_eq!(t(&s.right["trimStart"]), 10 * S);
        assert_eq!(t(&s.left_set["trimStart"]), 13 * S);
    }

    #[test]
    fn split_outside_is_refused() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "text", "startTime": 2, "endTime": 6}));
        assert!(split(&it, 2 * S, "b").is_none());
        assert!(split(&it, 6 * S, "b").is_none());
        assert!(split(&it, 7 * S, "b").is_none());
    }

    #[test]
    fn keyframes_are_cut_at_the_split() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "text", "startTime": 0, "endTime": 4,
            "keyframeTracks": [{"property": "opacity", "keyframes": [{"time": 0, "value": 0}, {"time": 4, "value": 1}]}]}));
        let s = split(&it, S, "b").expect("inside");
        let left = &s.left_set["keyframeTracks"][0]["keyframes"];
        let right = &s.right["keyframeTracks"][0]["keyframes"];
        assert_eq!(left.as_array().map(Vec::len), Some(2));
        assert_eq!(t(&left[1]["time"]), S);
        assert_eq!(left[1]["value"].as_f64(), Some(0.25));
        assert_eq!(t(&right[0]["time"]), 0);
        assert_eq!(right[0]["value"].as_f64(), Some(0.25));
        assert_eq!(t(&right[1]["time"]), 3 * S);
    }

    #[test]
    fn a_split_keeps_per_keyframe_easing_and_hold() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "text", "startTime": 0, "endTime": 4,
            "keyframeTracks": [{"property": "opacity", "keyframes": [
                {"time": 0, "value": 1, "easing": "hold"}, {"time": 2, "value": 0, "easing": "ease-in"}, {"time": 4, "value": 1}]}]}));
        let s = split(&it, S, "b").expect("inside");
        let left = &s.left_set["keyframeTracks"][0]["keyframes"];
        let right = &s.right["keyframeTracks"][0]["keyframes"];
        assert_eq!(left[0]["easing"], "hold");
        assert_eq!(left[1]["value"].as_f64(), Some(1.0), "the hold value at the cut");
        assert_eq!(right[0]["easing"], "hold", "the cut keyframe holds like the segment it cut");
        assert_eq!(right[1]["easing"], "ease-in");
        // The right part still shows 1 until its next keyframe, then 0.
        let r = item(json!({"id": "b", "trackId": "v", "type": "text", "startTime": 1, "endTime": 4,
            "keyframeTracks": [{"property": "opacity", "keyframes": right}]}));
        assert_eq!(property_at(&r, "opacity", 1_900_000), Some(1.0));
        assert_eq!(property_at(&r, "opacity", 2 * S), Some(0.0));
    }

    #[test]
    fn edge_fields_go_to_their_side() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "video", "startTime": 0, "endTime": 4,
            "mediaItemId": "m", "fadeIn": 0.5, "fadeOut": 0.5,
            "transitionIn": {"type": "fade", "duration": 0.5}, "transitionOut": {"type": "fade", "duration": 0.5}}));
        let s = split(&it, S, "b").expect("inside");
        assert!(s.left_unset.contains(&"fadeOut".to_string()));
        assert!(s.left_unset.contains(&"transitionOut".to_string()));
        assert!(s.right.get("fadeIn").is_none() && s.right.get("transitionIn").is_none());
        assert!(s.right.get("fadeOut").is_some() && s.right.get("transitionOut").is_some());
    }

    #[test]
    fn caption_segments_follow_the_cut() {
        let it = item(json!({"id": "c", "trackId": "cap", "type": "caption", "startTime": 10, "endTime": 14,
            "captionSegments": [
                {"id": "s1", "text": "one two", "startTime": 0, "endTime": 2,
                 "wordTimings": [{"word": "one", "start": 0, "end": 1}, {"word": "two", "start": 1, "end": 2}]},
                {"id": "s2", "text": "three", "startTime": 2.5, "endTime": 4}]}));
        let s = split(&it, 11 * S, "c2").expect("inside");
        let left = s.left_set["captionSegments"].as_array().expect("array");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0]["wordTimings"].as_array().map(Vec::len), Some(1));
        let right = s.right["captionSegments"].as_array().expect("array");
        assert_eq!(right.len(), 2);
        assert_eq!(t(&right[0]["startTime"]), 0);
        assert_eq!(right[0]["wordTimings"][0]["word"], "two");
        assert_eq!(t(&right[1]["startTime"]), 1_500_000);
    }

    #[test]
    fn rebase_start_keeps_keyframe_times_on_the_timeline() {
        let it = item(json!({"id": "a", "trackId": "v", "type": "text", "startTime": 0, "endTime": 4,
            "keyframeTracks": [{"property": "opacity", "keyframes": [{"time": 2, "value": 1}, {"time": 3, "value": 0}]}]}));
        let later = rebase_start(&it, S);
        assert_eq!(t(&later["keyframeTracks"][0]["keyframes"][0]["time"]), S);
        assert!(rebase_start(&it, 0).is_empty(), "no move, no change");
    }

    #[test]
    fn split_at_double_speed() {
        // Spec scenario "Split at double speed": in point 0, split 3 s after the start, right in point 6 s.
        let it = item(json!({"id": "a", "trackId": "v", "type": "video", "startTime": 1, "endTime": 9,
            "mediaItemId": "m", "playbackSpeed": 2}));
        let s = split(&it, 4 * S, "b").expect("inside");
        assert_eq!(t(&s.right["trimStart"]), 6 * S);
    }

    #[test]
    fn reverse_trim_of_the_start_keeps_frames_in_place() {
        // Spec scenario "Reverse item trim": the remaining timeline positions show the same frames.
        let it = item(json!({"id": "a", "trackId": "v", "type": "video", "startTime": 0, "endTime": 4,
            "trimStart": 2, "mediaItemId": "m", "playbackDirection": "reverse"}));
        let trimmed = cut(&it, S, 4 * S, Edges::TRIM);
        // Source shown at timeline time t (reverse): trimStart + (end - t) × speed.
        let shown = |i: &Item, at: Micros| i.time_field("trimStart") + (i.end - at);
        for at in [S, 2 * S, 3 * S, 4 * S - 1] {
            assert_eq!(shown(&trimmed, at), shown(&it, at));
        }
    }
}
