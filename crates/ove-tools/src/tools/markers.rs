//! Timeline markers (add-markers-keyframe-lanes-and-versions, design D1 and D5): `list_markers`, `add_marker`,
//! `update_marker` and `remove_markers`. Agents pick up work by listing markers by status and assignee, and hand it
//! back by setting a marker to `review` after checking the edit with `look`. Only a person resolves a marker: a
//! request from an agent surface to set `resolved` is refused with `marker.resolveByAgent` and changes nothing.

use serde_json::{Map, Value, json};

use ove_core::model::{Document, Marker};
use ove_core::ops::Op;
use ove_core::schema::{MARKER_NOTE_MAX, MARKER_STATUSES};
use ove_core::time::{micros_to_json, secs_to_micros};

use crate::ctx::{ToolCtx, edit, read, via};
use crate::fail::{Fail, plural};
use crate::receipt::Receipt;
use crate::spec::Kind::{List, Num, Str};
use crate::spec::{Arg, Args, arg, req};

/// The review rule, shared by the tool descriptions and the assistant prompt.
pub const REVIEW_RULE: &str = "Markers carry review work: list open markers assigned to `agent`, make the edit, check it with `look`, then set the marker's status to `review`. Never set `resolved`: only the person resolves a marker, and the request is refused.";

pub const LIST_MARKERS_ABOUT: &str = "List the timeline markers in time order: id, time and duration in seconds, name, colour, note, status (open, review, resolved), assignee, and the ids of the items under each marker. Filter with `status` and `assignee` (for example open markers assigned to `agent`).";
pub const LIST_MARKERS_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    arg("status", Str, "Only markers with this status: open, review or resolved"),
    arg("assignee", Str, "Only markers assigned to this name (`agent`, `user`, …)"),
];

pub const ADD_MARKER_ABOUT: &str = "Add a marker at `at` seconds (a range with `dur`), as one revision. Status defaults to open. Markers carry review work: an agent never sets `resolved` (refused); see update_marker.";
pub const ADD_MARKER_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    req("at", Num, "Timeline time in seconds"),
    arg("dur", Num, "Duration in seconds (default 0: a point marker)"),
    arg("name", Str, "Short label"),
    arg("color", Str, "red, orange, yellow, green, teal, blue, purple, pink or #rrggbb"),
    arg("note", Str, "Free text, at most 4,000 characters"),
    arg("status", Str, "open (default), review or resolved"),
    arg("assignee", Str, "Who should act on it: `agent`, `user` or a name"),
];

pub const UPDATE_MARKER_ABOUT: &str = "Change one marker as one revision: time, duration, name, colour, note, status or assignee; `unset` removes name, color, note or assignee. Markers carry review work: list open markers assigned to `agent`, make the edit, check it with `look`, then set the marker's status to `review`. Never set `resolved`: only the person resolves a marker, and the request is refused with `marker.resolveByAgent`.";
pub const UPDATE_MARKER_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    req("id", Str, "Marker id (from list_markers)"),
    arg("at", Num, "New time in seconds"),
    arg("dur", Num, "New duration in seconds"),
    arg("name", Str, "New label"),
    arg("color", Str, "New colour"),
    arg("note", Str, "New note"),
    arg("status", Str, "open, review or resolved (resolved: people only)"),
    arg("assignee", Str, "New assignee"),
    arg("unset", List, "Fields to remove: name, color, note, assignee"),
];

pub const REMOVE_MARKERS_ABOUT: &str = "Remove markers by id as one revision.";
pub const REMOVE_MARKERS_ARGS: &[Arg] = &[crate::registry::PROJECT, req("ids", List, "Marker ids (from list_markers)")];

/// Marker ids for new markers, unique per process.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("mk_{:x}{}", crate::fail::now_ms(), N.fetch_add(1, Ordering::Relaxed))
}

/// Ids of the items under a marker: items that cover its time (a point) or overlap its range.
pub fn items_under(doc: &Document, m: &Marker) -> Vec<String> {
    let (a, b) = (m.at, m.end());
    doc.items_in_canonical_order()
        .filter(|i| if a == b { i.start <= a && a < i.end } else { i.start < b && a < i.end })
        .map(|i| i.id.clone())
        .collect()
}

/// A marker as the tools return it.
pub fn marker_json(doc: &Document, m: &Marker) -> Value {
    let mut v = m.to_json();
    v["items"] = json!(items_under(doc, m));
    v
}

fn line(m: &Marker) -> String {
    let mut s = format!("{}\t{:.3}s", m.id, m.at as f64 / 1e6);
    if m.dur > 0 {
        s.push_str(&format!("+{:.3}s", m.dur as f64 / 1e6));
    }
    s.push_str(&format!("\t{}", m.status));
    for k in ["assignee", "name", "note"] {
        if let Some(v) = m.str_field(k) {
            s.push_str(&format!("\t{k}: {}", v.replace('\n', " ")));
        }
    }
    s
}

fn check_status(ctx: &dyn ToolCtx, status: &str) -> Result<(), Fail> {
    if !MARKER_STATUSES.contains(&status) {
        return Err(Fail::new(
            "argument.invalid",
            format!("status must be one of {}, got \"{status}\"", MARKER_STATUSES.join(", ")),
        ));
    }
    if status == "resolved" && ctx.surface().is_agent() {
        return Err(Fail::new(
            "marker.resolveByAgent",
            "only the person resolves a marker; set it to review after checking the edit with look",
        ));
    }
    Ok(())
}

fn secs_arg(a: &Args, k: &str) -> Result<Option<Value>, Fail> {
    match a.f64(k) {
        None => Ok(None),
        Some(s) => secs_to_micros(s)
            .filter(|us| *us >= 0)
            .map(|us| Some(micros_to_json(us)))
            .ok_or_else(|| Fail::new("argument.invalid", format!("`{k}` must be a non-negative number of seconds"))),
    }
}

/// The fields a call sets, from its arguments (times normalised, status checked).
fn fields(ctx: &dyn ToolCtx, a: &Args) -> Result<Map<String, Value>, Fail> {
    let mut set = Map::new();
    for k in ["at", "dur"] {
        if let Some(v) = secs_arg(a, k)? {
            set.insert(k.into(), v);
        }
    }
    for k in ["name", "color", "note", "status", "assignee"] {
        if let Some(v) = a.str(k) {
            set.insert(k.into(), v.into());
        }
    }
    if let Some(s) = a.str("status") {
        check_status(ctx, s)?;
    }
    if a.str("note").is_some_and(|n| n.chars().count() > MARKER_NOTE_MAX) {
        return Err(Fail::new("marker.note.length", format!("a marker note has at most {MARKER_NOTE_MAX} characters")));
    }
    Ok(set)
}

pub fn list_markers(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let status = a.str("status");
    if let Some(s) = status
        && !MARKER_STATUSES.contains(&s)
    {
        return Err(Fail::new("argument.invalid", format!("status must be one of {}", MARKER_STATUSES.join(", "))));
    }
    let assignee = a.str("assignee");
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let d = p.doc();
        let shown: Vec<&Marker> = d
            .markers_in_order()
            .into_iter()
            .filter(|m| status.is_none_or(|s| m.status == s))
            .filter(|m| assignee.is_none_or(|w| m.str_field("assignee") == Some(w)))
            .collect();
        let list: Vec<Value> = shown.iter().map(|m| marker_json(d, m)).collect();
        let mut text = format!("{} (rev {})", plural(shown.len(), "marker"), p.rev());
        for m in &shown {
            text.push('\n');
            text.push_str(&line(m));
            let items = items_under(d, m);
            if !items.is_empty() {
                text.push_str(&format!("\titems: {}", items.join(", ")));
            }
        }
        Ok(Receipt::new(json!({"rev": p.rev(), "markers": list}), text))
    })
}

pub fn add_marker(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let mut body = fields(ctx, a)?;
    if !body.contains_key("at") {
        return Err(Fail::new("argument.missing", "add_marker needs `at`"));
    }
    let id = new_id();
    body.insert("id".into(), id.clone().into());
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let rev = w.apply(vec![Op::AddMarker { marker: Value::Object(body.clone()) }], "add marker".into())?;
        let m = w.doc().marker(&id).cloned().ok_or_else(|| Fail::new("internal", "the marker was not added"))?;
        let text = format!("rev {rev}: added marker {}{}", line(&m), via(w));
        Ok(Receipt::new(json!({"rev": rev, "marker": marker_json(w.doc(), &m)}), text))
    })
}

pub fn update_marker(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let id = a.req_str("id")?.to_string();
    let set = fields(ctx, a)?;
    let unset = a.strings("unset");
    for k in &unset {
        if !["name", "color", "note", "assignee"].contains(&k.as_str()) {
            return Err(Fail::new(
                "argument.invalid",
                format!("`{k}` cannot be removed; unset takes name, color, note, assignee"),
            ));
        }
    }
    if set.is_empty() && unset.is_empty() {
        return Err(Fail::new("argument.missing", "update_marker needs a field to change"));
    }
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        if w.doc().marker(&id).is_none() {
            return Err(Fail::new("marker.missing", format!("marker {id} does not exist")));
        }
        let what = if set.get("status").and_then(Value::as_str) == Some("review") {
            "marker to review"
        } else {
            "update marker"
        };
        let rev =
            w.apply(vec![Op::UpdateMarker { id: id.clone(), set: set.clone(), unset: unset.clone() }], what.into())?;
        let m = w.doc().marker(&id).cloned().ok_or_else(|| Fail::new("internal", "the marker is gone"))?;
        let text = format!("rev {rev}: {}{}", line(&m), via(w));
        Ok(Receipt::new(json!({"rev": rev, "marker": marker_json(w.doc(), &m)}), text))
    })
}

pub fn remove_markers(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let ids = a.strings("ids");
    if ids.is_empty() {
        return Err(Fail::new("argument.missing", "remove_markers needs `ids`"));
    }
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let missing: Vec<&String> = ids.iter().filter(|id| w.doc().marker(id).is_none()).collect();
        if !missing.is_empty() {
            return Err(Fail::new(
                "marker.missing",
                format!("no marker {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")),
            ));
        }
        let rev =
            w.apply(vec![Op::RemoveMarkers { ids: ids.clone() }], format!("remove {}", plural(ids.len(), "marker")))?;
        Ok(Receipt::new(
            json!({"rev": rev, "removed": ids}),
            format!("rev {rev}: removed {}{}", plural(ids.len(), "marker"), via(w)),
        ))
    })
}
