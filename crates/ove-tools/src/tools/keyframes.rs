//! `edit_keyframes` (add-markers-keyframe-lanes-and-versions task 3.4): set, add or remove the keyframes of one
//! property of one item as one revision, with per-keyframe easing (including `hold`). A keyframe outside the item's
//! time range is refused with `keyframe.outOfRange`.

use ove_edit::keyframes::{KeyEdit, KeySpec, parse_keys, plan_keyframes};
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, edit, via};
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Kind::{Bool, Json, Str};
use crate::spec::{Arg, Args, arg, req};

const KEYS: &str = r#"{"type": ["array", "string"], "items": {"type": "object", "properties": {"time": {"type": "number"}, "value": {"type": "number"}, "easing": {"type": "string", "enum": ["linear", "ease-in", "ease-out", "ease-in-out", "hold"]}}, "required": ["time", "value"], "additionalProperties": false}}"#;
const TIMES: &str = r#"{"type": ["array", "string"], "items": {"type": "number"}}"#;

pub const EDIT_KEYFRAMES_ABOUT: &str = "Set, add or remove the keyframes of one property of one item, as one revision. Properties: x, y, scale, rotation, opacity, gaussianBlur, borderRadius, volume (sound items, 0-200), crop.top, crop.right, crop.bottom, crop.left (percent). Times are seconds from the item's start and must lie inside the item (`keyframe.outOfRange` otherwise). Each keyframe may set `easing` for the segment that starts at it (linear, ease-in, ease-out, ease-in-out, hold: keep the value until the next keyframe). `set` replaces the list (empty removes it), `add` adds or replaces keyframes at their times, `remove` takes times.";
pub const EDIT_KEYFRAMES_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    req("item", Str, "Item id"),
    req("property", Str, "The keyframed property, e.g. opacity or crop.left"),
    arg("set", Json(KEYS), "Every keyframe: [{time, value, easing?}] or \"0:1,2:0\""),
    arg("add", Json(KEYS), "Keyframes to add: [{time, value, easing?}] or \"1:0.5:hold\""),
    arg("remove", Json(TIMES), "Times of the keyframes to remove (seconds from the item's start)"),
    arg("easing", Str, "The track's easing for segments without their own"),
    arg("preview", Bool, "Return the plan without applying it"),
];

fn keys(v: &Value, what: &str) -> Result<Vec<KeySpec>, Fail> {
    match v {
        Value::String(s) => parse_keys(s).map_err(|r| Fail::new(r.code, r.message)),
        other => serde_json::from_value(other.clone())
            .map_err(|e| Fail::new("argument.invalid", format!("`{what}` is not a list of keyframes: {e}"))),
    }
}

fn times(v: &Value) -> Result<Vec<i64>, Fail> {
    let list: Vec<f64> = match v {
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(|x| x.parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| Fail::new("argument.invalid", "`remove` takes times in seconds, e.g. 1,2.5"))?,
        other => serde_json::from_value(other.clone())
            .map_err(|_| Fail::new("argument.invalid", "`remove` takes times in seconds"))?,
    };
    list.into_iter()
        .map(|s| ove_core::time::secs_to_micros(s).ok_or_else(|| Fail::new("argument.invalid", "times must be finite")))
        .collect()
}

/// The requested edit: exactly one of `set`, `add`, `remove`.
pub fn key_edit(a: &Args) -> Result<KeyEdit, Fail> {
    let given: Vec<&str> = ["set", "add", "remove"].into_iter().filter(|k| a.value(k).is_some()).collect();
    match given.as_slice() {
        ["set"] => Ok(KeyEdit::Set(keys(a.value("set").unwrap_or(&Value::Null), "set")?)),
        ["add"] => Ok(KeyEdit::Add(keys(a.value("add").unwrap_or(&Value::Null), "add")?)),
        ["remove"] => Ok(KeyEdit::Remove(times(a.value("remove").unwrap_or(&Value::Null))?)),
        [] if a.str("easing").is_some() => Ok(KeyEdit::Add(vec![])),
        _ => Err(Fail::new("argument.invalid", "give exactly one of `set`, `add` or `remove`")),
    }
}

pub fn edit_keyframes(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let item = a.req_str("item")?.to_string();
    let property = a.req_str("property")?.to_string();
    let ke = key_edit(a)?;
    let easing = a.str("easing").map(str::to_string);
    let preview = a.bool("preview");
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let plan = plan_keyframes(w.doc(), &item, &property, &ke, easing.as_deref())
            .map_err(|r| Fail::new(r.code.clone(), r.message.clone()).details(json!({"culprits": r.culprits})))?;
        let track = ove_core::retime::find_track(
            plan.result.item(&item).and_then(|i| i.fields.get("keyframeTracks")),
            &property,
        )
        .cloned();
        let mut v =
            json!({"item": item, "property": property, "summary": plan.summary, "track": track, "dryRun": preview});
        if preview || plan.ops.is_empty() {
            v["rev"] = w.rev().into();
            let what = if plan.ops.is_empty() { "no change" } else { "would apply" };
            return Ok(Receipt::new(v, format!("{what}: {}", plan.summary)));
        }
        let rev = w.apply(plan.ops.clone(), plan.summary.clone())?;
        v["rev"] = rev.into();
        let text = format!("rev {rev}: {} on {item}{}", plan.summary, via(w));
        Ok(Receipt::new(v, text))
    })
}
