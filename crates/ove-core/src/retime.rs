//! Item retiming shared by split and trim.
//!
//! Moving an item's start edge or cutting it in two changes three kinds of field:
//! - the source mapping (`trimStart`, `trimEnd`) of video and audio, which depends on `playbackSpeed` and
//!   `playbackDirection`;
//! - fields stored relative to the item start: keyframe times and caption segments with their word timings;
//! - fields tied to one edge: fades and transitions.
//!
//! The functions here return item fields only; callers turn them into ops so each edit stays one revision.

use serde_json::{Map, Value, json};

use crate::model::Item;
use crate::time::{Micros, json_to_micros, micros_to_json};

/// Fields that belong to the start edge of an item.
const START_EDGE: &[&str] = &["fadeIn", "audioFadeIn", "transitionIn"];
/// Fields that belong to the end edge of an item.
const END_EDGE: &[&str] = &["fadeOut", "audioFadeOut", "transitionOut"];

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

fn speed(item: &Item) -> f64 {
    item.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01)
}

fn reversed(item: &Item) -> bool {
    item.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse")
}

fn has_source(item: &Item) -> bool {
    matches!(item.kind.as_str(), "video" | "audio")
}

fn scaled(us: Micros, speed: f64) -> Micros {
    (us as f64 * speed).round() as Micros
}

/// Cuts `item` at timeline time `cut` (strictly inside the item). The left part keeps the id and the start-edge
/// fields; the right part gets `right_id` and the end-edge fields. Both parts show the same source frames and the
/// same animated values at every timeline time as the uncut item. Returns `None` when `cut` is not inside the item.
pub fn split(item: &Item, cut: Micros, right_id: &str) -> Option<Split> {
    if cut <= item.start || cut >= item.end {
        return None;
    }
    let left_len = cut - item.start;
    let right_len = item.end - cut;
    let mut left_set = Map::new();
    let mut left_unset = Vec::new();
    let mut right = item.to_json();
    right["id"] = right_id.into();
    right["startTime"] = micros_to_json(cut);
    left_set.insert("endTime".into(), micros_to_json(cut));

    if has_source(item) {
        let s = speed(item);
        let trim_start = item.time_field("trimStart");
        let trim_end = item.fields.get("trimEnd").and_then(json_to_micros);
        // Forward playback: the left part plays the earlier source. Reverse playback: the right part does.
        let (left_extra_start, right_extra_start, left_extra_end, right_extra_end) = if reversed(item) {
            (scaled(right_len, s), 0, 0, scaled(left_len, s))
        } else {
            (0, scaled(left_len, s), scaled(right_len, s), 0)
        };
        if left_extra_start != 0 {
            left_set.insert("trimStart".into(), micros_to_json(trim_start + left_extra_start));
        }
        right["trimStart"] = micros_to_json(trim_start + right_extra_start);
        if let Some(te) = trim_end {
            if left_extra_end != 0 {
                left_set.insert("trimEnd".into(), micros_to_json(te + left_extra_end));
            }
            right["trimEnd"] = micros_to_json(te + right_extra_end);
        }
    }

    // Item-relative fields.
    if let Some(tracks) = item.fields.get("keyframeTracks").and_then(Value::as_array) {
        left_set.insert("keyframeTracks".into(), Value::Array(window_keyframes(tracks, 0, left_len)));
        right["keyframeTracks"] = Value::Array(window_keyframes(tracks, left_len, item.end - item.start));
    }
    if let Some(segs) = item.fields.get("captionSegments").and_then(Value::as_array) {
        left_set.insert("captionSegments".into(), Value::Array(window_segments(segs, 0, left_len)));
        right["captionSegments"] = Value::Array(window_segments(segs, left_len, item.end - item.start));
    }

    // Edge fields.
    for k in END_EDGE {
        if item.fields.contains_key(*k) {
            left_unset.push((*k).to_string());
        }
    }
    if let Some(obj) = right.as_object_mut() {
        for k in START_EDGE {
            obj.remove(*k);
        }
    }
    Some(Split { left_set, left_unset, right })
}

/// Fields to set when the start edge of `item` moves to `new_start` (the source mapping is the caller's: see
/// `trimmed` in the desktop app). Shifts keyframes and caption segments so they stay at the same timeline times;
/// keyframes and segments that fall before the new start are cut at it.
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
fn track_value(keyframes: &[(Micros, f64)], easing: &str, t: Micros) -> Option<f64> {
    let (&first, &last) = (keyframes.first()?, keyframes.last()?);
    if t <= first.0 {
        return Some(first.1);
    }
    if t >= last.0 {
        return Some(last.1);
    }
    let i = keyframes.iter().position(|k| k.0 > t)?;
    let (a, b) = (keyframes[i - 1], keyframes[i]);
    let p = (t - a.0) as f64 / (b.0 - a.0).max(1) as f64;
    Some(a.1 + (b.1 - a.1) * ease(easing, p))
}

/// Keyframe easing curves; the renderer uses this same function.
pub fn ease(easing: &str, p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    match easing {
        "ease-in" => p * p,
        "ease-out" => 1.0 - (1.0 - p) * (1.0 - p),
        "ease-in-out" => {
            if p < 0.5 {
                2.0 * p * p
            } else {
                1.0 - (-2.0 * p + 2.0).powi(2) / 2.0
            }
        }
        _ => p,
    }
}

/// Keyframe tracks restricted to the item-relative window `[from, to]` and shifted so `from` becomes 0. A keyframe is
/// added at each window edge that falls between keyframes, holding the value there, so the curve is unchanged
/// inside the window for linear tracks (eased tracks keep their end values; the shape between is re-eased).
fn window_keyframes(tracks: &[Value], from: Micros, to: Micros) -> Vec<Value> {
    tracks
        .iter()
        .map(|track| {
            let mut out = track.clone();
            let easing = track.get("easing").and_then(Value::as_str).unwrap_or("linear");
            let Some(list) = track.get("keyframes").and_then(Value::as_array) else {
                return out;
            };
            let mut kfs: Vec<(Micros, f64)> = list
                .iter()
                .filter_map(|k| Some((k.get("time").and_then(json_to_micros)?, k.get("value")?.as_f64()?)))
                .collect();
            kfs.sort_by_key(|k| k.0);
            let mut kept: Vec<(Micros, f64)> = Vec::new();
            if let Some(v) = track_value(&kfs, easing, from)
                && kfs.first().is_some_and(|f| f.0 < from)
            {
                kept.push((from, v));
            }
            kept.extend(kfs.iter().copied().filter(|k| k.0 >= from && k.0 <= to));
            if let Some(v) = track_value(&kfs, easing, to)
                && kfs.last().is_some_and(|l| l.0 > to)
            {
                kept.push((to, v));
            }
            kept.dedup_by_key(|k| k.0);
            out["keyframes"] = Value::Array(
                kept.into_iter().map(|(t, v)| json!({"time": micros_to_json(t - from), "value": v})).collect(),
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
}
