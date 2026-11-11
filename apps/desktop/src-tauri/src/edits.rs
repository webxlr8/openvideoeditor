//! Editing commands beyond import/move/split: generic field edits, adding items (titles, captions, adjustment
//! layers), duplicate, copy/paste, trim with source sync, and track management. Everything is one revision through
//! the document pipeline (`ove-store` → `ove-core` ops), validated by the document invariants.

use std::sync::atomic::{AtomicU64, Ordering};

use ove_core::Item;
use ove_core::ops::Op;
use ove_core::time::{Micros, json_to_micros, micros_to_json, secs_to_micros};
use ove_store::project::Project;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, State};

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock};

/// Shortest item an edit may leave (one frame at 25 fps).
const MIN_DURATION: Micros = 40_000;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

fn parse<T: for<'de> Deserialize<'de>>(json: &str, what: &str) -> AppResult<T> {
    serde_json::from_str(json).map_err(|e| bad(format!("{what} is not valid: {e}")))
}

/// New unique item id.
pub fn new_item_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock) as u64;
    format!("i_{:x}{}", now, N.fetch_add(1, Ordering::Relaxed))
}

fn with_project<R>(state: &AppState, f: impl FnOnce(&Project) -> AppResult<R>) -> AppResult<R> {
    let g = lock(&state.projects.open)?;
    let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
    f(p)
}

fn item(p: &Project, id: &str) -> AppResult<Item> {
    p.doc().item(id).cloned().ok_or_else(|| AppError::new(ErrorCode::NotFound, "That item no longer exists."))
}

/// Full JSON of an item (every field, document names) for the inspector.
#[tauri::command]
#[specta::specta]
pub fn item_detail(state: State<'_, AppState>, id: String) -> AppResult<String> {
    with_project(&state, |p| Ok(item(p, &id)?.to_json().to_string()))
}

#[derive(Deserialize)]
struct ItemEdit {
    id: String,
    #[serde(default)]
    set: Map<String, Value>,
    #[serde(default)]
    unset: Vec<String>,
}

fn edit_ops(edits: Vec<ItemEdit>) -> Vec<Op> {
    let mut ops = Vec::new();
    for e in edits {
        if !e.unset.is_empty() {
            ops.push(Op::UnsetItemFields { id: e.id.clone(), fields: e.unset });
        }
        if !e.set.is_empty() {
            ops.push(Op::UpdateItem { id: e.id, set: e.set });
        }
    }
    ops
}

/// Sets and unsets fields of one item (`set_json`: an object of document field names → values).
#[tauri::command]
#[specta::specta]
pub fn update_item(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    set_json: String,
    unset: Vec<String>,
    summary: Option<String>,
) -> AppResult<ProjectView> {
    let set: Map<String, Value> = parse(&set_json, "the change")?;
    let ops = edit_ops(vec![ItemEdit { id, set, unset }]);
    edit(&app, &state, ops, summary.as_deref().unwrap_or("edit item"))
}

/// Several item edits as one revision: `[{id, set: {...}, unset: [...]}, …]`.
#[tauri::command]
#[specta::specta]
pub fn update_items(
    app: AppHandle,
    state: State<'_, AppState>,
    edits_json: String,
    summary: Option<String>,
) -> AppResult<ProjectView> {
    let edits: Vec<ItemEdit> = parse(&edits_json, "the changes")?;
    let n = edits.len();
    let summary = summary.unwrap_or_else(|| format!("edit {n} item{}", if n == 1 { "" } else { "s" }));
    edit(&app, &state, edit_ops(edits), &summary)
}

fn prepare_new(mut v: Value) -> AppResult<Value> {
    let o = v.as_object_mut().ok_or_else(|| bad("An item must be an object."))?;
    if !o.get("id").is_some_and(Value::is_string) {
        o.insert("id".into(), new_item_id().into());
    }
    for k in ["trackId", "type", "startTime", "endTime"] {
        if !o.contains_key(k) {
            return Err(bad(format!("A new item needs {k}.")));
        }
    }
    Ok(v)
}

/// Adds items (titles, captions, adjustment layers, …). Missing ids are generated.
#[tauri::command]
#[specta::specta]
pub fn add_items(
    app: AppHandle,
    state: State<'_, AppState>,
    items_json: String,
    summary: Option<String>,
) -> AppResult<ProjectView> {
    let items: Vec<Value> = parse(&items_json, "the new items")?;
    let n = items.len();
    let ops =
        items.into_iter().map(|i| prepare_new(i).map(|item| Op::AddItem { item })).collect::<AppResult<Vec<_>>>()?;
    let summary = summary.unwrap_or_else(|| format!("add {n} item{}", if n == 1 { "" } else { "s" }));
    edit(&app, &state, ops, &summary)
}

/// Free span on a track starting at `from` for `dur`, else the track's end.
fn free_start(p: &Project, track: &str, from: Micros, dur: Micros, ignore: &[String]) -> Micros {
    let items: Vec<&Item> = p.doc().items_on_track(track).into_iter().filter(|i| !ignore.contains(&i.id)).collect();
    let overlaps = |s: Micros| items.iter().any(|i| i.start < s + dur && s < i.end);
    if !overlaps(from) {
        return from;
    }
    items.iter().map(|i| i.end).max().unwrap_or(0).max(from)
}

/// Duplicates items right after themselves on their track (or at the track's end when that is taken).
#[tauri::command]
#[specta::specta]
pub fn duplicate_items(app: AppHandle, state: State<'_, AppState>, ids: Vec<String>) -> AppResult<ProjectView> {
    let ops = with_project(&state, |p| {
        let mut ops = Vec::new();
        for id in &ids {
            let it = item(p, id)?;
            let dur = it.end - it.start;
            let at = free_start(p, &it.track_id, it.end, dur, &[]);
            let mut j = it.to_json();
            j["id"] = new_item_id().into();
            j["startTime"] = micros_to_json(at);
            j["endTime"] = micros_to_json(at + dur);
            ops.push(Op::AddItem { item: j });
        }
        Ok(ops)
    })?;
    edit(&app, &state, ops, "duplicate")
}

/// Pastes copied items (full item JSON as from `item_detail`) so the earliest starts at `at` seconds, keeping
/// relative timing and tracks. Items whose track no longer exists go to the first track of the same kind.
#[tauri::command]
#[specta::specta]
pub fn paste_items(app: AppHandle, state: State<'_, AppState>, items_json: String, at: f64) -> AppResult<ProjectView> {
    let items: Vec<Value> = parse(&items_json, "the clipboard")?;
    let at = secs_to_micros(at.max(0.0)).unwrap_or(0);
    let ops = with_project(&state, |p| {
        let min = items.iter().filter_map(|i| i.get("startTime").and_then(json_to_micros)).min().unwrap_or(0);
        let mut ops = Vec::new();
        for mut i in items {
            let (Some(s), Some(e)) =
                (i.get("startTime").and_then(json_to_micros), i.get("endTime").and_then(json_to_micros))
            else {
                continue;
            };
            let track = i.get("trackId").and_then(Value::as_str).unwrap_or_default().to_string();
            if p.doc().track(&track).is_none() {
                let kind = i.get("type").and_then(Value::as_str).unwrap_or("video");
                let want = if kind == "audio" {
                    "audio"
                } else if kind == "caption" {
                    "caption"
                } else {
                    "video"
                };
                match p.doc().project.tracks.iter().find(|t| t.kind == want) {
                    Some(t) => i["trackId"] = t.id.clone().into(),
                    None => continue,
                }
            }
            if i.get("mediaItemId").and_then(Value::as_str).is_some_and(|m| !p.doc().project.media.contains_key(m)) {
                continue;
            }
            i["id"] = new_item_id().into();
            i["startTime"] = micros_to_json(at + (s - min));
            i["endTime"] = micros_to_json(at + (e - min));
            ops.push(Op::AddItem { item: i });
        }
        Ok(ops)
    })?;
    if ops.is_empty() {
        return Err(bad("Nothing to paste here."));
    }
    edit(&app, &state, ops, "paste")
}

/// The trim of one edge, in microseconds: `(start, end, trimStart)` after moving `edge` to `time`, keeping the
/// source in sync (moving the start edge moves the in-point by the same amount × speed) and within the source.
pub fn trimmed(
    it: &Item,
    media_duration: Option<Micros>,
    edge: &str,
    time: Micros,
) -> Result<(Micros, Micros, Micros), String> {
    let speed = it.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01);
    let has_source = it.media_id().is_some() && matches!(it.kind.as_str(), "video" | "audio");
    let trim = it.time_field("trimStart");
    match edge {
        "start" => {
            let mut start = time.clamp(0, it.end - MIN_DURATION);
            if has_source {
                // Cannot reveal source before its first frame.
                let earliest = it.start - (trim as f64 / speed).round() as Micros;
                start = start.max(earliest);
                let new_trim = trim + ((start - it.start) as f64 * speed).round() as Micros;
                Ok((start, it.end, new_trim.max(0)))
            } else {
                Ok((start, it.end, trim))
            }
        }
        "end" => {
            let mut end = time.max(it.start + MIN_DURATION);
            if has_source && let Some(d) = media_duration {
                let latest = it.start + ((d - trim).max(0) as f64 / speed).round() as Micros;
                end = end.min(latest.max(it.start + MIN_DURATION));
            }
            Ok((it.start, end, trim))
        }
        other => Err(format!("unknown edge {other}")),
    }
}

/// Trims an item's start or end edge to `time` (seconds on the timeline).
#[tauri::command]
#[specta::specta]
pub fn trim_item(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    edge: String,
    time: f64,
) -> AppResult<ProjectView> {
    let set = with_project(&state, |p| {
        let it = item(p, &id)?;
        let dur = it
            .media_id()
            .and_then(|m| p.doc().project.media.get(m))
            .and_then(|m| m.data().and_then(|d| d.get("duration")).and_then(json_to_micros));
        let t = secs_to_micros(time).ok_or_else(|| bad("The time is invalid."))?;
        let (s, e, tr) = trimmed(&it, dur, &edge, t).map_err(bad)?;
        let mut set = Map::new();
        set.insert("startTime".into(), micros_to_json(s));
        set.insert("endTime".into(), micros_to_json(e));
        if tr != it.time_field("trimStart") {
            set.insert("trimStart".into(), micros_to_json(tr));
        }
        Ok(set)
    })?;
    edit(&app, &state, vec![Op::UpdateItem { id, set }], "trim")
}

/// Adds a track. Video/caption/adjustment/layout tracks go on top, audio tracks at the bottom.
#[tauri::command]
#[specta::specta]
pub fn add_track(app: AppHandle, state: State<'_, AppState>, kind: String, name: String) -> AppResult<ProjectView> {
    let (order, id) = with_project(&state, |p| {
        let n = p.doc().project.tracks.len();
        let id = format!("t_{}", new_item_id().trim_start_matches("i_"));
        Ok((if kind == "audio" { n } else { 0 }, id))
    })?;
    edit(
        &app,
        &state,
        vec![Op::AddTrack { track: json!({"id": id, "name": name, "order": order, "type": kind}) }],
        "add track",
    )
}

#[tauri::command]
#[specta::specta]
pub fn rename_track(app: AppHandle, state: State<'_, AppState>, id: String, name: String) -> AppResult<ProjectView> {
    let mut set = Map::new();
    set.insert("name".into(), name.into());
    edit(&app, &state, vec![Op::UpdateTrack { id, set }], "rename track")
}

/// New track order, top first: every track id once.
#[tauri::command]
#[specta::specta]
pub fn reorder_tracks(app: AppHandle, state: State<'_, AppState>, order: Vec<String>) -> AppResult<ProjectView> {
    edit(&app, &state, vec![Op::ReorderTracks { order }], "reorder tracks")
}

/// Removes a track and everything on it.
#[tauri::command]
#[specta::specta]
pub fn remove_track(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<ProjectView> {
    let ids =
        with_project(&state, |p| Ok(p.doc().items_on_track(&id).iter().map(|i| i.id.clone()).collect::<Vec<_>>()))?;
    let mut ops = Vec::new();
    if !ids.is_empty() {
        ops.push(Op::RemoveItems { ids });
    }
    ops.push(Op::RemoveTrack { id });
    edit(&app, &state, ops, "remove track")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::revision::Source;
    use ove_store::Library;

    fn clip(start: f64, end: f64, trim: f64, speed: f64) -> Item {
        Item::from_json(
            &json!({"id": "c", "trackId": "t", "type": "video", "startTime": start, "endTime": end,
                    "trimStart": trim, "mediaItemId": "m", "playbackSpeed": speed}),
            "item",
        )
        .unwrap()
    }

    #[test]
    fn trim_start_moves_the_in_point() {
        let c = clip(10.0, 20.0, 2.0, 1.0);
        assert_eq!(trimmed(&c, Some(30_000_000), "start", 12_000_000).unwrap(), (12_000_000, 20_000_000, 4_000_000));
        // Cannot go before the first source frame: in-point 2 s → earliest start 8 s.
        assert_eq!(trimmed(&c, Some(30_000_000), "start", 5_000_000).unwrap(), (8_000_000, 20_000_000, 0));
        // At 2× speed, 1 s of timeline is 2 s of source.
        let fast = clip(10.0, 20.0, 2.0, 2.0);
        assert_eq!(trimmed(&fast, None, "start", 11_000_000).unwrap(), (11_000_000, 20_000_000, 4_000_000));
    }

    #[test]
    fn trim_end_stays_within_the_source() {
        let c = clip(10.0, 20.0, 2.0, 1.0);
        // Source is 15 s, in-point 2 s → at most 13 s of clip.
        assert_eq!(trimmed(&c, Some(15_000_000), "end", 40_000_000).unwrap(), (10_000_000, 23_000_000, 2_000_000));
        assert_eq!(trimmed(&c, Some(15_000_000), "end", 9_000_000).unwrap().1, 10_000_000 + MIN_DURATION);
        assert!(trimmed(&c, None, "middle", 0).is_err());
    }

    #[test]
    fn ops_apply_through_the_pipeline() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), crate::projects::clock()).unwrap();
        let mut p = lib.create("Edits").unwrap();
        p.apply(crate::projects::default_tracks(), Source::Gui, None).unwrap();
        let title = prepare_new(
            json!({"trackId": "t_text", "type": "text", "startTime": 1, "endTime": 4, "textContent": "Hello"}),
        )
        .unwrap();
        p.apply(vec![Op::AddItem { item: title.clone() }], Source::Gui, None).unwrap();
        let id = title["id"].as_str().unwrap().to_string();
        let mut set = Map::new();
        set.insert("opacity".into(), json!(0.5));
        p.apply(edit_ops(vec![ItemEdit { id: id.clone(), set, unset: vec!["textContent".into()] }]), Source::Gui, None)
            .unwrap();
        let it = p.doc().item(&id).unwrap();
        assert_eq!(it.fields["opacity"], json!(0.5));
        assert!(!it.fields.contains_key("textContent"));
        assert_eq!(free_start(&p, "t_text", 2_000_000, 1_000_000, &[]), 4_000_000);
        assert_eq!(free_start(&p, "t_text", 5_000_000, 1_000_000, &[]), 5_000_000);
        // An invalid edit is refused by the invariants.
        let mut bad_set = Map::new();
        bad_set.insert("opacity".into(), json!(3));
        assert!(p.apply(edit_ops(vec![ItemEdit { id, set: bad_set, unset: vec![] }]), Source::Gui, None).is_err());
    }
}
