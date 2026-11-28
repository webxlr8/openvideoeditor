//! Editing commands beyond import/move/split: generic field edits, adding items (titles, captions, adjustment
//! layers), duplicate, copy/paste, trim with source sync, and track management. Everything is one revision through
//! the document pipeline (`ove-store` → `ove-core` ops), validated by the document invariants.

use std::sync::atomic::{AtomicU64, Ordering};

use ove_core::Item;
use ove_core::ops::Op;
use ove_core::time::{Micros, json_to_micros, micros_to_json, secs_to_micros};
use ove_store::project::Project;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};

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
pub async fn item_detail(app: AppHandle, id: String) -> AppResult<String> {
    serial(&app, "item_detail", move |_, state| with_project(state, |p| Ok(item(p, &id)?.to_json().to_string()))).await
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
pub async fn update_item(
    app: AppHandle,
    id: String,
    set_json: String,
    unset: Vec<String>,
    summary: Option<String>,
) -> AppResult<ProjectView> {
    serial(&app, "update_item", move |app, state| {
        let set: Map<String, Value> = parse(&set_json, "the change")?;
        let ops = edit_ops(vec![ItemEdit { id, set, unset }]);
        edit(app, state, ops, summary.as_deref().unwrap_or("edit item"))
    })
    .await
}

/// Several item edits as one revision: `[{id, set: {...}, unset: [...]}, …]`.
#[tauri::command]
#[specta::specta]
pub async fn update_items(app: AppHandle, edits_json: String, summary: Option<String>) -> AppResult<ProjectView> {
    serial(&app, "update_items", move |app, state| {
        let edits: Vec<ItemEdit> = parse(&edits_json, "the changes")?;
        let n = edits.len();
        let summary = summary.unwrap_or_else(|| format!("edit {n} item{}", if n == 1 { "" } else { "s" }));
        edit(app, state, edit_ops(edits), &summary)
    })
    .await
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
pub async fn add_items(app: AppHandle, items_json: String, summary: Option<String>) -> AppResult<ProjectView> {
    serial(&app, "add_items", move |app, state| {
        let items: Vec<Value> = parse(&items_json, "the new items")?;
        let n = items.len();
        let ops = items
            .into_iter()
            .map(|i| prepare_new(i).map(|item| Op::AddItem { item }))
            .collect::<AppResult<Vec<_>>>()?;
        let summary = summary.unwrap_or_else(|| format!("add {n} item{}", if n == 1 { "" } else { "s" }));
        edit(app, state, ops, &summary)
    })
    .await
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

/// New link values for copies: copied items that shared a link share a new one (the copies are a new group, not
/// members of the original's); a copy whose partners were not copied is no longer linked.
fn relink(items: &mut [Value]) {
    let mut count: std::collections::BTreeMap<String, usize> = Default::default();
    for i in items.iter() {
        if let Some(l) = i.get("link").and_then(Value::as_str) {
            *count.entry(l.to_string()).or_default() += 1;
        }
    }
    let fresh: std::collections::BTreeMap<String, String> = count
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(l, _)| (l, format!("l_{}", new_item_id().trim_start_matches("i_"))))
        .collect();
    for i in items.iter_mut() {
        let Some(o) = i.as_object_mut() else { continue };
        match o.get("link").and_then(Value::as_str).and_then(|l| fresh.get(l)).cloned() {
            Some(l) => {
                o.insert("link".into(), l.into());
            }
            None => {
                o.shift_remove("link");
            }
        }
    }
}

/// Duplicates items right after themselves on their track (or at the track's end when that is taken).
#[tauri::command]
#[specta::specta]
pub async fn duplicate_items(app: AppHandle, ids: Vec<String>) -> AppResult<ProjectView> {
    serial(&app, "duplicate_items", move |app, state| {
        let ops = with_project(state, |p| {
            let mut copies = Vec::new();
            for id in &ids {
                let it = item(p, id)?;
                let dur = it.end - it.start;
                let at = free_start(p, &it.track_id, it.end, dur, &[]);
                let mut j = it.to_json();
                j["id"] = new_item_id().into();
                j["startTime"] = micros_to_json(at);
                j["endTime"] = micros_to_json(at + dur);
                copies.push(j);
            }
            relink(&mut copies);
            Ok(copies.into_iter().map(|item| Op::AddItem { item }).collect::<Vec<_>>())
        })?;
        edit(app, state, ops, "duplicate")
    })
    .await
}

/// Pastes copied items (full item JSON as from `item_detail`) so the earliest starts at `at` seconds, keeping
/// relative timing and tracks. Items whose track no longer exists go to the first track of the same kind.
#[tauri::command]
#[specta::specta]
pub async fn paste_items(app: AppHandle, items_json: String, at: f64) -> AppResult<ProjectView> {
    serial(&app, "paste_items", move |app, state| {
        let mut items: Vec<Value> = parse(&items_json, "the clipboard")?;
        relink(&mut items);
        let at = secs_to_micros(at.max(0.0)).unwrap_or(0);
        let ops = with_project(state, |p| {
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
                if i.get("mediaItemId").and_then(Value::as_str).is_some_and(|m| !p.doc().project.media.contains_key(m))
                {
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
        edit(app, state, ops, "paste")
    })
    .await
}

/// Plans an edit with the shared planners and applies it as one revision. A refusal changes nothing and becomes
/// the command's error (its message names what blocked the edit); a plan with no ops returns the current view.
pub(crate) fn plan_and_apply(app: &AppHandle, state: &AppState, req: ove_edit::Request) -> AppResult<ProjectView> {
    let plan = with_project(state, |p| {
        let mut plan = ove_edit::plan_with(p.doc(), &req, &mut new_item_id).map_err(|r| {
            let code = if r.code == "item.missing" { ErrorCode::NotFound } else { ErrorCode::InvalidArgument };
            AppError::new(code, r.message)
        })?;
        // In the editor, markers follow ripples (add-markers-keyframe-lanes-and-versions D4).
        ove_edit::markers::follow(p.doc(), &req, &mut plan);
        Ok(plan)
    })?;
    if plan.is_empty() {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
        return Ok(crate::projects::cached_view(app, state, p));
    }
    if plan.time_map.is_empty() {
        return edit(app, state, plan.ops, &plan.summary);
    }
    // A ripple records how it moved time, so a projection printed before it rebases onto it.
    let mut g = lock(&state.projects.open)?;
    let p = g.as_mut().ok_or_else(|| bad("No project is open."))?;
    p.apply_mapped(plan.ops, ove_core::revision::Source::Gui, Some(plan.summary), plan.time_map)?;
    crate::projects::emit_view(app, Some(p)).ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
}

fn parse_request(json: &str) -> AppResult<ove_edit::Request> {
    parse(json, "the edit request")
}

/// Applies one edit verb (an `ove-edit` request as JSON: `{"op": "rippleDelete", "items": ["…"]}`, times in seconds)
/// as one revision. A refusal is the error and changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn edit_plan(app: AppHandle, request: String) -> AppResult<ProjectView> {
    let req = parse_request(&request)?;
    serial(&app, "edit_plan", move |app, state| plan_and_apply(app, state, req)).await
}

/// An item where an edit would put it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Ghost {
    pub id: String,
    pub track_id: String,
    pub start: f64,
    pub end: f64,
}

/// Why an edit would be refused.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EditRefusal {
    pub code: String,
    /// Items to outline.
    pub culprits: Vec<String>,
    pub message: String,
}

/// What an edit would do, without doing it: the items it adds or changes at their new place (for ghost
/// rendering), the items it removes, the items that stopped at a limit, or why it would be refused.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EditPreview {
    pub summary: String,
    pub ghosts: Vec<Ghost>,
    pub removed: Vec<String>,
    pub limited: Vec<String>,
    pub refusal: Option<EditRefusal>,
}

/// Plans an edit verb without applying it (no revision): what a drag would do while it is held.
#[tauri::command]
#[specta::specta]
pub async fn edit_preview(app: AppHandle, request: String) -> AppResult<EditPreview> {
    let req = parse_request(&request)?;
    crate::worker::blocking("edit_preview", move || {
        let state = app.state::<AppState>();
        with_project(&state, |p| Ok(preview(p.doc(), &req)))
    })
    .await
}

/// The preview of a request on a document.
pub fn preview(doc: &ove_core::Document, req: &ove_edit::Request) -> EditPreview {
    let secs = |us: Micros| us as f64 / 1e6;
    match ove_edit::plan_with(doc, req, &mut new_item_id) {
        Err(r) => EditPreview {
            summary: String::new(),
            ghosts: vec![],
            removed: vec![],
            limited: vec![],
            refusal: Some(EditRefusal { code: r.code, culprits: r.culprits, message: r.message }),
        },
        Ok(plan) => {
            let mut ghosts = Vec::new();
            let mut removed = Vec::new();
            for id in &plan.touched {
                match plan.result.item(id) {
                    Some(it) => ghosts.push(Ghost {
                        id: id.clone(),
                        track_id: it.track_id.clone(),
                        start: secs(it.start),
                        end: secs(it.end),
                    }),
                    None => removed.push(id.clone()),
                }
            }
            EditPreview { summary: plan.summary, ghosts, removed, limited: plan.limited, refusal: None }
        }
    }
}

/// Trims an item's start or end edge to `time` (seconds on the timeline), keeping the source in sync and within the
/// source. Linked items trim with it unless `linked` is false (Alt in the timeline: a J or L cut). The trimmed edge
/// may overlap a neighbour (an overlap plays as a crossfade).
#[tauri::command]
#[specta::specta]
pub async fn trim_item(
    app: AppHandle,
    id: String,
    edge: String,
    time: f64,
    linked: Option<bool>,
) -> AppResult<ProjectView> {
    let edge = match edge.as_str() {
        "start" => ove_edit::Edge::Start,
        "end" => ove_edit::Edge::End,
        other => return Err(bad(format!("unknown edge {other}"))),
    };
    let to = secs_to_micros(time).ok_or_else(|| bad("The time is invalid."))?;
    serial(&app, "trim_item", move |app, state| {
        let linked = linked.unwrap_or(true);
        plan_and_apply(app, state, ove_edit::Request::Trim { item: id, edge, to, linked, crossfade: true })
    })
    .await
}

/// Adds a track. Video/caption/adjustment/layout tracks go on top, audio tracks at the bottom.
#[tauri::command]
#[specta::specta]
pub async fn add_track(app: AppHandle, kind: String, name: String) -> AppResult<ProjectView> {
    serial(&app, "add_track", move |app, state| {
        let (order, id) = with_project(state, |p| {
            let n = p.doc().project.tracks.len();
            let id = format!("t_{}", new_item_id().trim_start_matches("i_"));
            Ok((if kind == "audio" { n } else { 0 }, id))
        })?;
        edit(
            app,
            state,
            vec![Op::AddTrack { track: json!({"id": id, "name": name, "order": order, "type": kind}) }],
            "add track",
        )
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_track(app: AppHandle, id: String, name: String) -> AppResult<ProjectView> {
    serial(&app, "rename_track", move |app, state| {
        let mut set = Map::new();
        set.insert("name".into(), name.into());
        edit(app, state, vec![Op::UpdateTrack { id, set }], "rename track")
    })
    .await
}

/// New track order, top first: every track id once.
#[tauri::command]
#[specta::specta]
pub async fn reorder_tracks(app: AppHandle, order: Vec<String>) -> AppResult<ProjectView> {
    serial(&app, "reorder_tracks", move |app, state| {
        edit(app, state, vec![Op::ReorderTracks { order }], "reorder tracks")
    })
    .await
}

/// Removes a track and everything on it.
#[tauri::command]
#[specta::specta]
pub async fn remove_track(app: AppHandle, id: String) -> AppResult<ProjectView> {
    serial(&app, "remove_track", move |app, state| {
        let ids =
            with_project(state, |p| Ok(p.doc().items_on_track(&id).iter().map(|i| i.id.clone()).collect::<Vec<_>>()))?;
        let mut ops = Vec::new();
        if !ids.is_empty() {
            ops.push(Op::RemoveItems { ids });
        }
        ops.push(Op::RemoveTrack { id });
        edit(app, state, ops, "remove track")
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::revision::Source;
    use ove_store::Library;

    #[test]
    fn preview_shows_where_items_go_and_changes_nothing() {
        let mut d = ove_core::Document::new("p", "Preview", 1);
        d = ove_core::ops::apply_batch(
            &d,
            &[
                Op::AddTrack { track: json!({"id": "v", "name": "V", "order": 0, "type": "video"}) },
                Op::AddTrack { track: json!({"id": "a", "name": "A", "order": 1, "type": "audio"}) },
                Op::AddItem {
                    item: json!({"id": "x", "trackId": "v", "type": "text", "startTime": 0, "endTime": 2, "link": "L"}),
                },
                Op::AddItem {
                    item: json!({"id": "y", "trackId": "a", "type": "text", "startTime": 0, "endTime": 2, "link": "L"}),
                },
            ],
        )
        .unwrap()
        .doc;
        let before = d.clone();
        let req: ove_edit::Request = serde_json::from_value(json!({"op": "move", "item": "x", "start": 3})).unwrap();
        let p = preview(&d, &req);
        assert!(p.refusal.is_none());
        let mut ghosts: Vec<(String, f64)> = p.ghosts.iter().map(|g| (g.id.clone(), g.start)).collect();
        ghosts.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(ghosts, vec![("x".to_string(), 3.0), ("y".to_string(), 3.0)]);
        assert_eq!(d, before);
        let req: ove_edit::Request = serde_json::from_value(json!({"op": "split", "items": ["x"], "at": 9})).unwrap();
        assert_eq!(preview(&d, &req).refusal.unwrap().code, "edit.outside");
    }

    #[test]
    fn copies_get_their_own_link_group() {
        let mut items = vec![
            json!({"id": "v", "link": "L"}),
            json!({"id": "a", "link": "L"}),
            json!({"id": "w", "link": "M"}),
            json!({"id": "t"}),
        ];
        relink(&mut items);
        assert_eq!(items[0]["link"], items[1]["link"]);
        assert_ne!(items[0]["link"], json!("L"));
        assert!(items[2].get("link").is_none(), "a copy without its partner is not linked");
        assert!(items[3].get("link").is_none());
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
