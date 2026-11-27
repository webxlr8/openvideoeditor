//! Timeline markers in the editor (add-markers-keyframe-lanes-and-versions task 2.5): the markers the ruler lane
//! shows, and the commands behind the M key, the marker popover and its delete button. Each command is one revision
//! through the document pipeline. A person may resolve a marker; agents cannot (the tool layer refuses them).

use std::sync::atomic::{AtomicU64, Ordering};

use ove_core::Document;
use ove_core::ops::Op;
use ove_core::time::{micros_to_json, secs_to_micros};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, serial};

/// A marker as the timeline shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarkerView {
    pub id: String,
    /// Seconds.
    pub at: f64,
    /// Seconds; 0 for a point marker.
    pub dur: f64,
    /// open, review or resolved.
    pub status: String,
    pub name: Option<String>,
    pub color: Option<String>,
    pub note: Option<String>,
    pub assignee: Option<String>,
}

/// The markers of a document in time order, or `None` when there are none (the view leaves the field out).
pub fn views(d: &Document) -> Option<Vec<MarkerView>> {
    if d.project.markers.is_empty() {
        return None;
    }
    let s = |m: &ove_core::Marker, k: &str| m.str_field(k).map(str::to_string);
    Some(
        d.markers_in_order()
            .into_iter()
            .map(|m| MarkerView {
                id: m.id.clone(),
                at: m.at as f64 / 1e6,
                dur: m.dur as f64 / 1e6,
                status: m.status.clone(),
                name: s(m, "name"),
                color: s(m, "color"),
                note: s(m, "note"),
                assignee: s(m, "assignee"),
            })
            .collect(),
    )
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

/// New unique marker id.
pub fn new_marker_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock) as u64;
    format!("mk_{:x}{}", now, N.fetch_add(1, Ordering::Relaxed))
}

/// Converts `at` and `dur` (seconds) in a change to document times.
fn normalise(set: &mut Map<String, Value>) -> AppResult<()> {
    for k in ["at", "dur"] {
        if let Some(v) = set.get(k) {
            let us = v
                .as_f64()
                .and_then(secs_to_micros)
                .filter(|us| *us >= 0)
                .ok_or_else(|| bad(format!("{k} must be a non-negative number of seconds")))?;
            set.insert(k.into(), micros_to_json(us));
        }
    }
    Ok(())
}

fn parse_fields(json: &str) -> AppResult<Map<String, Value>> {
    serde_json::from_str(json).map_err(|e| bad(format!("the marker change is not valid: {e}")))
}

/// Adds a marker at `at` seconds (the M key adds a point marker at the playhead). `fields_json` may set dur, name,
/// color, note, status and assignee.
#[tauri::command]
#[specta::specta]
pub async fn marker_add(app: AppHandle, at: f64, fields_json: Option<String>) -> AppResult<ProjectView> {
    serial(&app, "marker_add", move |app, state| {
        let mut m = match &fields_json {
            Some(j) => parse_fields(j)?,
            None => Map::new(),
        };
        m.insert("at".into(), at.into());
        normalise(&mut m)?;
        m.insert("id".into(), new_marker_id().into());
        edit(app, state, vec![Op::AddMarker { marker: Value::Object(m) }], "add marker")
    })
    .await
}

/// Changes a marker (`set_json`: at, dur, name, color, note, status, assignee) and removes optional fields.
#[tauri::command]
#[specta::specta]
pub async fn marker_update(app: AppHandle, id: String, set_json: String, unset: Vec<String>) -> AppResult<ProjectView> {
    serial(&app, "marker_update", move |app, state| {
        let mut set = parse_fields(&set_json)?;
        normalise(&mut set)?;
        let summary = match set.get("status").and_then(Value::as_str) {
            Some("resolved") => "resolve marker",
            Some("review") => "marker to review",
            _ => "edit marker",
        };
        edit(app, state, vec![Op::UpdateMarker { id, set, unset }], summary)
    })
    .await
}

/// Removes markers.
#[tauri::command]
#[specta::specta]
pub async fn marker_remove(app: AppHandle, ids: Vec<String>) -> AppResult<ProjectView> {
    serial(&app, "marker_remove", move |app, state| {
        let n = ids.len();
        edit(app, state, vec![Op::RemoveMarkers { ids }], if n == 1 { "remove marker" } else { "remove markers" })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::apply_batch;
    use serde_json::json;

    #[test]
    fn views_are_in_time_order_and_absent_without_markers() {
        let d = Document::new("p", "x", 1);
        assert!(views(&d).is_none());
        let d = apply_batch(
            &d,
            &[
                Op::AddMarker { marker: json!({"id": "b", "at": 3, "name": "Late"}) },
                Op::AddMarker { marker: json!({"id": "a", "at": 1.5, "dur": 2, "status": "review"}) },
            ],
        )
        .unwrap()
        .doc;
        let v = views(&d).unwrap();
        assert_eq!(v.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!((v[0].at, v[0].dur, v[0].status.as_str()), (1.5, 2.0, "review"));
        assert_eq!(v[1].name.as_deref(), Some("Late"));
    }

    #[test]
    fn times_are_seconds_on_the_wire() {
        let mut m = Map::new();
        m.insert("at".into(), json!(2.5));
        normalise(&mut m).unwrap();
        assert_eq!(m["at"], json!(2.5));
        m.insert("dur".into(), json!(-1));
        assert!(normalise(&mut m).is_err());
    }
}
