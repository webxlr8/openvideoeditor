//! Keyframe easing and evaluation shared by the renderer, the audio mix and the edit planners.
//!
//! The split and trim retiming that lived here moved to the `ove-edit` crate (`ove_edit::retime`), where every edit
//! planner shares it.

use serde_json::Value;

use crate::time::{Micros, json_to_micros};

/// Keyframe easing curves; the renderer uses this same function. `hold` keeps the start value until the next
/// keyframe.
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
        "hold" => {
            if p >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        _ => p,
    }
}

/// One keyframe: item-relative time, value, and the easing of the segment that starts at it (when it overrides the
/// track's).
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub time: Micros,
    pub value: f64,
    pub easing: Option<String>,
}

/// The keyframes of a keyframe track (`{property, keyframes, easing}`), sorted by time. Malformed keyframes are
/// skipped.
pub fn keys(track: &Value) -> Vec<Key> {
    let mut v: Vec<Key> = track
        .get("keyframes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|k| {
                    Some(Key {
                        time: k.get("time").and_then(json_to_micros)?,
                        value: k.get("value")?.as_f64()?,
                        easing: k.get("easing").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|k| k.time);
    v
}

/// The track's own easing (`linear` when absent).
pub fn track_easing(track: &Value) -> &str {
    track.get("easing").and_then(Value::as_str).unwrap_or("linear")
}

/// Value of sorted keyframes at item-relative time `t`: each segment uses its start keyframe's easing, else the
/// track easing. Before the first keyframe the first value holds, after the last the last value. `None` when there
/// are no keyframes.
pub fn value_at(keys: &[Key], track_easing: &str, t: Micros) -> Option<f64> {
    let (first, last) = (keys.first()?, keys.last()?);
    if t <= first.time {
        return Some(first.value);
    }
    if t >= last.time {
        return Some(last.value);
    }
    // first.time < t < last.time, so the position is found and is at least 1.
    let i = keys.iter().position(|k| k.time > t)?;
    let (a, b) = (&keys[i - 1], &keys[i]);
    let p = (t - a.time) as f64 / (b.time - a.time).max(1) as f64;
    Some(a.value + (b.value - a.value) * ease(a.easing.as_deref().unwrap_or(track_easing), p))
}

/// Value of a keyframe track at item-relative time `t`.
pub fn track_value(track: &Value, t: Micros) -> Option<f64> {
    value_at(&keys(track), track_easing(track), t)
}

/// The track for `property` in an item's `keyframeTracks` value, if any.
pub fn find_track<'a>(tracks: Option<&'a Value>, property: &str) -> Option<&'a Value> {
    tracks?.as_array()?.iter().find(|t| t.get("property").and_then(Value::as_str) == Some(property))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn easing_ends_are_fixed() {
        for e in ["linear", "ease-in", "ease-out", "ease-in-out", "hold"] {
            assert_eq!(ease(e, 0.0), 0.0);
            assert_eq!(ease(e, 1.0), 1.0);
        }
        assert_eq!(ease("ease-in", 0.5), 0.25);
        assert_eq!(ease("hold", 0.999), 0.0);
    }

    #[test]
    fn hold_keeps_the_value_until_the_next_keyframe() {
        // Spec scenario: opacity 1.0 at 0 s with hold, 0.0 at 2 s.
        let t = json!({"property": "opacity", "keyframes": [{"time": 0, "value": 1, "easing": "hold"}, {"time": 2, "value": 0}]});
        assert_eq!(track_value(&t, 1_900_000), Some(1.0));
        assert_eq!(track_value(&t, 2_000_000), Some(0.0));
    }

    #[test]
    fn a_keyframe_easing_overrides_the_track_easing_for_its_segment() {
        let t = json!({"property": "x", "easing": "ease-in", "keyframes": [
            {"time": 0, "value": 0, "easing": "linear"}, {"time": 1, "value": 10}, {"time": 2, "value": 20}]});
        assert_eq!(track_value(&t, 500_000), Some(5.0), "first segment: linear");
        assert_eq!(track_value(&t, 1_500_000), Some(12.5), "second segment: the track's ease-in");
    }

    #[test]
    fn crop_keyframes_interpolate() {
        // Spec scenario: left crop 0 at 0 s and 0.25 at 1 s, linear.
        let t = json!({"property": "crop.left", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": 0.25}]});
        assert_eq!(track_value(&t, 500_000), Some(0.125));
    }
}
