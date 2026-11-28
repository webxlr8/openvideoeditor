//! Keyframe lanes (add-markers-keyframe-lanes-and-versions task 3.3): the keyframes the timeline's lanes show, and the
//! command behind each lane gesture (add, drag, delete). A gesture sends the property's whole new keyframe list and
//! becomes one revision; expanding a lane is view state and changes nothing. Volume keyframes keep their gain line on
//! the clip, so they are not repeated here.

use ove_core::Item;
use ove_edit::keyframes::{KeyEdit, KeySpec, plan_keyframes};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};

/// One keyframe as a lane shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    /// Seconds from the item's start.
    pub time: f64,
    pub value: f64,
    /// The easing of the segment that starts here, when it overrides the track's.
    pub easing: Option<String>,
}

/// One keyframed property of an item.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyframeTrackView {
    pub property: String,
    /// The track's easing (`linear` when not set).
    pub easing: String,
    pub keys: Vec<KeyView>,
}

/// The item's keyframe tracks other than volume, or `None` when it has none.
pub fn views(item: &Item) -> Option<Vec<KeyframeTrackView>> {
    let tracks = item.fields.get("keyframeTracks")?.as_array()?;
    let out: Vec<KeyframeTrackView> = tracks
        .iter()
        .filter_map(|t| {
            let property = t.get("property").and_then(Value::as_str)?;
            (property != "volume").then(|| KeyframeTrackView {
                property: property.to_string(),
                easing: ove_core::retime::track_easing(t).to_string(),
                keys: ove_core::retime::keys(t)
                    .into_iter()
                    .map(|k| KeyView { time: k.time as f64 / 1e6, value: k.value, easing: k.easing })
                    .collect(),
            })
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

/// Replaces the keyframes of one property of an item (`keys_json`: `[{time, value, easing?}]`, seconds from the
/// item's start; an empty list removes the property's keyframes) as one revision.
#[tauri::command]
#[specta::specta]
pub async fn keyframes_set(
    app: AppHandle,
    item: String,
    property: String,
    keys_json: String,
    summary: Option<String>,
) -> AppResult<ProjectView> {
    serial(&app, "keyframes_set", move |app, state| {
        let keys: Vec<KeySpec> =
            serde_json::from_str(&keys_json).map_err(|e| bad(format!("the keyframes are not valid: {e}")))?;
        let plan = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
            plan_keyframes(p.doc(), &item, &property, &KeyEdit::Set(keys), None).map_err(|r| {
                let code = if r.code == "item.missing" { ErrorCode::NotFound } else { ErrorCode::InvalidArgument };
                AppError::new(code, r.message)
            })?
        };
        let summary = summary.unwrap_or(plan.summary);
        edit(app, state, plan.ops, &summary)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn views_leave_out_volume_and_keep_easings() {
        let item = Item::from_json(
            &json!({"id": "a", "trackId": "t", "type": "audio", "startTime": 0, "endTime": 4, "keyframeTracks": [
                {"property": "volume", "keyframes": [{"time": 0, "value": 100}]},
                {"property": "opacity", "easing": "ease-in", "keyframes": [{"time": 1, "value": 0, "easing": "hold"}, {"time": 0, "value": 1}]}]}),
            "item",
        )
        .unwrap();
        let v = views(&item).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].property, "opacity");
        assert_eq!(v[0].easing, "ease-in");
        assert_eq!(v[0].keys[0], KeyView { time: 0.0, value: 1.0, easing: None });
        assert_eq!(v[0].keys[1].easing.as_deref(), Some("hold"));
        let plain =
            Item::from_json(&json!({"id": "b", "trackId": "t", "type": "text", "startTime": 0, "endTime": 1}), "i")
                .unwrap();
        assert!(views(&plain).is_none());
    }
}
