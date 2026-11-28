//! Keyframe edits on one property of one item (add-markers-keyframe-lanes-and-versions task 3.4): set the whole
//! list, add keyframes, or remove keyframes at given times, as one revision. Times are seconds from the item's start;
//! a keyframe outside the item (before 0 or after its length) is refused with `keyframe.outOfRange`. The command line,
//! MCP and the timeline's keyframe lanes all plan through here.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::schema::{KEYFRAME_EASINGS, KEYFRAME_PROPERTIES};
use ove_core::time::{Micros, micros_to_json};

use crate::{Plan, Refusal};

/// One keyframe as requested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySpec {
    /// Seconds from the item's start.
    #[serde(with = "crate::secs")]
    pub time: Micros,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<String>,
}

/// What to do with the property's keyframes.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyEdit {
    /// Replace every keyframe (an empty list removes the property's track).
    Set(Vec<KeySpec>),
    /// Add keyframes; one at the time of an existing keyframe replaces it.
    Add(Vec<KeySpec>),
    /// Remove the keyframes at these times.
    Remove(Vec<Micros>),
}

/// Parses `0:1,2:0` (seconds:value, optionally `:easing`) into keyframes.
pub fn parse_keys(s: &str) -> Result<Vec<KeySpec>, Refusal> {
    let bad = |part: &str| {
        Refusal::new(
            "argument.invalid",
            vec![],
            format!("`{part}` is not a keyframe: write seconds:value or seconds:value:easing, for example 0:1,2:0"),
        )
    };
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|part| {
            let mut it = part.split(':');
            let t: f64 = it.next().and_then(|x| x.trim().parse().ok()).ok_or_else(|| bad(part))?;
            let value: f64 = it.next().and_then(|x| x.trim().parse().ok()).ok_or_else(|| bad(part))?;
            let easing = it.next().map(|e| e.trim().to_string());
            if it.next().is_some() || !t.is_finite() || !value.is_finite() {
                return Err(bad(part));
            }
            let time = ove_core::time::secs_to_micros(t).ok_or_else(|| bad(part))?;
            Ok(KeySpec { time, value, easing })
        })
        .collect()
}

fn refuse(code: &str, item: &str, message: impl Into<String>) -> Refusal {
    Refusal::new(code, vec![item.to_string()], message)
}

/// Plans a keyframe edit. `track_easing` sets the track's own easing (`None` keeps it).
pub fn plan_keyframes(
    doc: &Document,
    item_id: &str,
    property: &str,
    edit: &KeyEdit,
    track_easing: Option<&str>,
) -> Result<Plan, Refusal> {
    let item =
        doc.item(item_id).ok_or_else(|| refuse("item.missing", item_id, format!("item {item_id} does not exist")))?;
    if !KEYFRAME_PROPERTIES.contains(&property) {
        return Err(refuse(
            "keyframe.property",
            item_id,
            format!("`{property}` cannot be keyframed; use one of {}", KEYFRAME_PROPERTIES.join(", ")),
        ));
    }
    if property == "volume" && !matches!(item.kind.as_str(), "video" | "audio") {
        return Err(refuse("keyframe.property", item_id, format!("item {item_id} has no sound to keyframe")));
    }
    let len = item.end - item.start;
    let check_easing = |e: &str| {
        if KEYFRAME_EASINGS.contains(&e) {
            Ok(())
        } else {
            Err(refuse(
                "keyframe.easing",
                item_id,
                format!("easing must be one of {}, got \"{e}\"", KEYFRAME_EASINGS.join(", ")),
            ))
        }
    };
    if let Some(e) = track_easing {
        check_easing(e)?;
    }
    let specs: &[KeySpec] = match edit {
        KeyEdit::Set(k) | KeyEdit::Add(k) => k,
        KeyEdit::Remove(_) => &[],
    };
    for k in specs {
        if k.time < 0 || k.time > len {
            return Err(refuse(
                "keyframe.outOfRange",
                item_id,
                format!(
                    "a keyframe at {}s is outside item {item_id}, which is {}s long (times are seconds from its start)",
                    k.time as f64 / 1e6,
                    len as f64 / 1e6
                ),
            ));
        }
        if let Some(e) = &k.easing {
            check_easing(e)?;
        }
    }
    let tracks: Vec<Value> = item.fields.get("keyframeTracks").and_then(Value::as_array).cloned().unwrap_or_default();
    let at = tracks.iter().position(|t| t.get("property").and_then(Value::as_str) == Some(property));
    let old = at.map(|i| tracks[i].clone());
    let mut keys = old.as_ref().map(ove_core::retime::keys).unwrap_or_default();
    match edit {
        KeyEdit::Set(k) => keys = k.iter().map(to_key).collect(),
        KeyEdit::Add(k) => {
            for s in k {
                keys.retain(|x| x.time != s.time);
                keys.push(to_key(s));
            }
        }
        KeyEdit::Remove(times) => {
            let missing: Vec<String> = times
                .iter()
                .filter(|t| !keys.iter().any(|k| k.time == **t))
                .map(|t| format!("{}s", *t as f64 / 1e6))
                .collect();
            if !missing.is_empty() {
                return Err(refuse(
                    "keyframe.missing",
                    item_id,
                    format!("item {item_id} has no {property} keyframe at {}", missing.join(", ")),
                ));
            }
            keys.retain(|k| !times.contains(&k.time));
        }
    }
    keys.sort_by_key(|k| k.time);
    keys.dedup_by_key(|k| k.time);

    let mut new_tracks = tracks.clone();
    if keys.is_empty() {
        if let Some(i) = at {
            new_tracks.remove(i);
        }
    } else {
        let mut t = old.clone().unwrap_or_else(|| json!({"property": property}));
        t["keyframes"] = Value::Array(keys.iter().map(key_json).collect());
        if let Some(e) = track_easing {
            t["easing"] = e.into();
        }
        match at {
            Some(i) => new_tracks[i] = t,
            None => new_tracks.push(t),
        }
    }
    if new_tracks == tracks {
        return Ok(Plan {
            ops: vec![],
            summary: "no change".into(),
            touched: vec![],
            time_map: Default::default(),
            limited: vec![],
            result: doc.clone(),
        });
    }
    let op = if new_tracks.is_empty() {
        Op::UnsetItemFields { id: item_id.to_string(), fields: vec!["keyframeTracks".into()] }
    } else {
        let mut set = Map::new();
        set.insert("keyframeTracks".into(), Value::Array(new_tracks));
        Op::UpdateItem { id: item_id.to_string(), set }
    };
    let applied = apply_batch(doc, std::slice::from_ref(&op))
        .map_err(|e| Refusal::new(e.code, vec![item_id.to_string()], e.message))?;
    let n = keys.len();
    let summary = match edit {
        KeyEdit::Set(_) if n == 0 => format!("remove {property} keyframes"),
        KeyEdit::Set(_) => format!("set {n} {property} keyframe{}", if n == 1 { "" } else { "s" }),
        KeyEdit::Add(k) => format!("add {} {property} keyframe{}", k.len(), if k.len() == 1 { "" } else { "s" }),
        KeyEdit::Remove(t) => format!("remove {} {property} keyframe{}", t.len(), if t.len() == 1 { "" } else { "s" }),
    };
    Ok(Plan {
        ops: vec![op],
        summary,
        touched: vec![item_id.to_string()],
        time_map: Default::default(),
        limited: vec![],
        result: applied.doc,
    })
}

fn to_key(s: &KeySpec) -> ove_core::retime::Key {
    ove_core::retime::Key { time: s.time, value: s.value, easing: s.easing.clone() }
}

fn key_json(k: &ove_core::retime::Key) -> Value {
    let mut v = json!({"time": micros_to_json(k.time), "value": k.value});
    if let Some(e) = &k.easing {
        v["easing"] = e.clone().into();
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Micros = 1_000_000;

    fn doc() -> Document {
        apply_batch(
            &Document::new("p", "K", 1),
            &[
                Op::AddTrack { track: json!({"id": "t", "name": "T", "order": 0, "type": "video"}) },
                Op::AddItem {
                    item: json!({"id": "i_1", "trackId": "t", "type": "text", "startTime": 1, "endTime": 4}),
                },
            ],
        )
        .unwrap()
        .doc
    }

    fn keys_of(d: &Document, prop: &str) -> Vec<(Micros, f64)> {
        let t = ove_core::retime::find_track(d.item("i_1").unwrap().fields.get("keyframeTracks"), prop);
        t.map(ove_core::retime::keys).unwrap_or_default().iter().map(|k| (k.time, k.value)).collect()
    }

    #[test]
    fn set_add_remove_each_make_one_op() {
        let d = doc();
        let p = plan_keyframes(&d, "i_1", "opacity", &KeyEdit::Set(parse_keys("0:1,2:0").unwrap()), None).unwrap();
        assert_eq!(p.ops.len(), 1);
        assert_eq!(keys_of(&p.result, "opacity"), [(0, 1.0), (2 * S, 0.0)]);
        let p2 = plan_keyframes(&p.result, "i_1", "opacity", &KeyEdit::Add(parse_keys("1:0.5:hold").unwrap()), None)
            .unwrap();
        assert_eq!(keys_of(&p2.result, "opacity"), [(0, 1.0), (S, 0.5), (2 * S, 0.0)]);
        let p3 = plan_keyframes(&p2.result, "i_1", "opacity", &KeyEdit::Remove(vec![0, S]), Some("ease-in")).unwrap();
        assert_eq!(keys_of(&p3.result, "opacity"), [(2 * S, 0.0)]);
        let p4 = plan_keyframes(&p3.result, "i_1", "opacity", &KeyEdit::Set(vec![]), None).unwrap();
        assert!(p4.result.item("i_1").unwrap().fields.get("keyframeTracks").is_none());
    }

    #[test]
    fn refusals() {
        let d = doc();
        let code = |p: &str, e: KeyEdit| plan_keyframes(&d, "i_1", p, &e, None).unwrap_err().code;
        // The item is 3 s long: a keyframe 5 s after it ends is outside.
        assert_eq!(code("opacity", KeyEdit::Set(parse_keys("0:1,8:0").unwrap())), "keyframe.outOfRange");
        assert_eq!(code("opacity", KeyEdit::Add(parse_keys("-1:0").unwrap())), "keyframe.outOfRange");
        assert_eq!(code("wobble", KeyEdit::Set(parse_keys("0:1").unwrap())), "keyframe.property");
        assert_eq!(code("volume", KeyEdit::Set(parse_keys("0:1").unwrap())), "keyframe.property");
        assert_eq!(code("opacity", KeyEdit::Set(parse_keys("0:1:bounce").unwrap())), "keyframe.easing");
        assert_eq!(code("opacity", KeyEdit::Remove(vec![S])), "keyframe.missing");
        assert_eq!(code("crop.left", KeyEdit::Set(parse_keys("0:120").unwrap())), "value.range");
        assert!(parse_keys("0-1").is_err());
        assert_eq!(
            plan_keyframes(&d, "ghost", "opacity", &KeyEdit::Set(vec![]), None).unwrap_err().code,
            "item.missing"
        );
    }
}
