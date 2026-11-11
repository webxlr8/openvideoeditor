//! Diff and rebase (design D3/D4): the changes between the base revision and the file are applied onto the current
//! document item by item and field by field (objects one key deeper, arrays whole). For each changed field with base
//! value B, file value F and current value C: C == B → take F; C == F → nothing; otherwise a conflict resolved by the
//! policy and reported.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value};

use ove_core::model::{Document, Item, MediaItem, RegistryEntry, Track};
use ove_core::schema::{self, Kind};
use ove_core::time::micros_to_json;

use crate::normal;
use crate::parse::Pos;
use crate::read::{FileItem, FileModel, FileTrack, new_media_json};

/// Which side wins a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Prefer {
    #[default]
    File,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// Item or track id (`aspect` / `track order` for document-level values).
    pub id: String,
    /// Field path (`opacity`, `transform.x`), or `item` when one side removed the item.
    pub field: String,
    pub file_value: Option<Value>,
    pub editor_value: Option<Value>,
    pub kept: Prefer,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub id: String,
    pub name: String,
}

/// The merged target document and what happened.
#[derive(Debug, Clone)]
pub struct Merged {
    pub target: Document,
    pub conflicts: Vec<Conflict>,
    pub removed: Vec<Removed>,
    /// Source positions of file items/tracks by their (possibly newly assigned) id, for error reporting.
    pub positions: BTreeMap<String, Pos>,
    /// Ids assigned to new items.
    pub new_ids: Vec<String>,
}

pub type Path = (String, Option<String>);

pub fn path_str(p: &Path) -> String {
    match &p.1 {
        Some(k) => format!("{}.{k}", p.0),
        None => p.0.clone(),
    }
}

/// Item fields as a flat comparable map including head fields (trackId, type, startTime, endTime).
pub fn item_view(i: &Item) -> Map<String, Value> {
    let mut m = normal::item_fields(&i.kind, &i.fields);
    m.insert("trackId".into(), i.track_id.clone().into());
    m.insert("type".into(), i.kind.clone().into());
    m.insert("startTime".into(), micros_to_json(i.start));
    m.insert("endTime".into(), micros_to_json(i.end));
    m
}

fn file_item_view(f: &FileItem, track_id: &str) -> Map<String, Value> {
    let mut m = f.fields.clone();
    m.insert("trackId".into(), track_id.into());
    m.insert("type".into(), f.kind.clone().into());
    m.insert("startTime".into(), micros_to_json(f.start));
    m.insert("endTime".into(), micros_to_json(f.end));
    m
}

fn is_obj_field(kind: &str, field: &str) -> bool {
    schema::item_field(kind, field).is_some_and(|f| matches!(f.kind, Kind::Obj { .. }))
}

/// Field-level changes from `b` to `f`; known object fields are compared one key deeper.
pub fn changes(kind_b: &str, b: &Map<String, Value>, f: &Map<String, Value>) -> Vec<(Path, Option<Value>)> {
    let mut out = Vec::new();
    let keys: BTreeSet<&String> = b.keys().chain(f.keys()).collect();
    for k in keys {
        let (bv, fv) = (b.get(k), f.get(k));
        if bv == fv {
            continue;
        }
        let objectish = is_obj_field(kind_b, k) && bv.is_none_or(Value::is_object) && fv.is_none_or(Value::is_object);
        if objectish {
            let empty = Map::new();
            let bo = bv.and_then(Value::as_object).unwrap_or(&empty);
            let fo = fv.and_then(Value::as_object).unwrap_or(&empty);
            let sub: BTreeSet<&String> = bo.keys().chain(fo.keys()).collect();
            for s in sub {
                if bo.get(s) != fo.get(s) {
                    out.push(((k.clone(), Some(s.clone())), fo.get(s).cloned()));
                }
            }
        } else {
            out.push(((k.clone(), None), fv.cloned()));
        }
    }
    out
}

fn get(view: &Map<String, Value>, p: &Path) -> Option<Value> {
    match &p.1 {
        None => view.get(&p.0).cloned(),
        Some(k) => view.get(&p.0).and_then(|o| o.get(k)).cloned(),
    }
}

/// Sets (or removes, with `None`) a field path on an item.
fn set_path(item: &mut Item, p: &Path, v: Option<Value>) {
    match p.0.as_str() {
        "trackId" => item.track_id = v.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
        "type" => item.kind = v.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
        "startTime" => item.start = v.as_ref().and_then(ove_core::time::json_to_micros).unwrap_or(0),
        "endTime" => item.end = v.as_ref().and_then(ove_core::time::json_to_micros).unwrap_or(0),
        field => match (&p.1, v) {
            (None, Some(v)) => {
                item.fields.insert(field.to_string(), v);
            }
            (None, None) => {
                item.fields.shift_remove(field);
            }
            (Some(k), v) => {
                let obj = item.fields.entry(field.to_string()).or_insert_with(|| Value::Object(Map::new()));
                if !obj.is_object() {
                    *obj = Value::Object(Map::new());
                }
                let o = obj.as_object_mut().unwrap();
                match v {
                    Some(v) => {
                        o.insert(k.clone(), v);
                    }
                    None => {
                        o.shift_remove(k);
                    }
                }
                if o.is_empty() {
                    item.fields.shift_remove(field);
                }
            }
        },
    }
}

fn track_view(t: &Track) -> Map<String, Value> {
    let mut m = normal::track_fields(&t.fields);
    m.insert("name".into(), t.name.clone().into());
    m.insert("type".into(), t.kind.clone().into());
    m
}

fn file_track_view(t: &FileTrack) -> Map<String, Value> {
    let mut m = t.fields.clone();
    m.insert("name".into(), t.name.clone().into());
    m.insert("type".into(), t.kind.clone().into());
    m
}

fn set_track_field(t: &mut Track, k: &str, v: Option<Value>) {
    match k {
        "name" => t.name = v.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
        "type" => t.kind = v.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
        _ => match v {
            Some(v) => {
                t.fields.insert(k.to_string(), v);
            }
            None => {
                t.fields.shift_remove(k);
            }
        },
    }
}

/// Generates an id not used in any of the given documents or the reserved set.
fn fresh_id(prefix: &str, taken: &dyn Fn(&str) -> bool, seed: &mut u64) -> String {
    loop {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let id = format!("{prefix}{:08x}", (*seed >> 32) as u32);
        if !taken(&id) {
            return id;
        }
    }
}

struct Ctx {
    prefer: Prefer,
    conflicts: Vec<Conflict>,
}

impl Ctx {
    /// The 3-way rule for one value. Returns `Some(new)` when the target must change.
    fn decide(
        &mut self,
        id: &str,
        field: &str,
        b: Option<Value>,
        f: Option<Value>,
        c: Option<Value>,
    ) -> Option<Option<Value>> {
        if c == b {
            return Some(f);
        }
        if c == f {
            return None;
        }
        self.conflicts.push(Conflict {
            id: id.to_string(),
            field: field.to_string(),
            file_value: f.clone(),
            editor_value: c,
            kept: self.prefer,
            message: format!("{id}.{field} was changed both in the file and in the editor"),
        });
        match self.prefer {
            Prefer::File => Some(f),
            Prefer::Editor => None,
        }
    }
}

/// Inserts ids missing from `list` after their predecessor in `order` (or at the front).
fn insert_after_predecessors(list: &mut Vec<String>, order: &[String], missing: &[String]) {
    for id in order.iter().filter(|i| missing.contains(i)) {
        let pos = order.iter().position(|x| x == id).unwrap();
        let pred = order[..pos].iter().rev().find(|p| list.contains(p));
        let at = pred.map(|p| list.iter().position(|x| x == p).unwrap() + 1).unwrap_or(0);
        list.insert(at, id.clone());
    }
}

/// Merges a file model (printed from `base`) onto `current`.
pub fn merge(base: &Document, current: &Document, file: &FileModel, prefer: Prefer, now_ms: i64) -> Merged {
    let mut ctx = Ctx { prefer, conflicts: Vec::new() };
    let mut m = current.clone();
    let mut positions = BTreeMap::new();
    let mut seed = (now_ms as u64) ^ 0x9e37_79b9_7f4a_7c15 ^ (current.project.rev as u64);

    // New media registered by path.
    for nm in &file.new_media {
        if !m.project.media.contains_key(&nm.id) {
            let (mj, ej) = new_media_json(nm, now_ms);
            m.project.media.insert(nm.id.clone(), Arc::new(MediaItem::from_json(&mj, "media").unwrap()));
            m.registry.insert(nm.id.clone(), RegistryEntry::from_json(&ej, "entry").unwrap());
        }
    }

    // Aspect ratio.
    if let Some(fa) = file.aspect.as_ref().filter(|fa| **fa != base.project.aspect_ratio) {
        let b = Some(Value::String(base.project.aspect_ratio.clone()));
        let c = Some(Value::String(current.project.aspect_ratio.clone()));
        if let Some(Some(Value::String(v))) = ctx.decide("timeline", "aspect", b, Some(Value::String(fa.clone())), c) {
            m.project.aspect_ratio = v;
        }
    }

    // Tracks: ids for new tracks, field merge, removals, then order.
    let taken_track = |id: &str| {
        base.track(id).is_some()
            || current.track(id).is_some()
            || file.tracks.iter().any(|t| t.id.as_deref() == Some(id))
    };
    let mut file_track_ids: Vec<String> = Vec::new();
    for ft in &file.tracks {
        // A new track the editor already holds identically (same file applied twice) keeps that track's id.
        let already = || {
            current.project.tracks.iter().find(|ct| {
                base.track(&ct.id).is_none()
                    && !file_track_ids.contains(&ct.id)
                    && track_view(ct) == file_track_view(ft)
            })
        };
        let id = match &ft.id {
            Some(id) if base.track(id).is_some() => id.clone(),
            Some(id) if current.track(id).is_none() => id.clone(),
            _ => match already() {
                Some(ct) => ct.id.clone(),
                None => fresh_id("t_", &taken_track, &mut seed),
            },
        };
        positions.insert(id.clone(), ft.pos);
        file_track_ids.push(id);
    }
    for (ft, id) in file.tracks.iter().zip(&file_track_ids) {
        let fv = file_track_view(ft);
        match base.track(id) {
            Some(bt) => {
                let bv = track_view(bt);
                let Some(ci) = m.project.tracks.iter().position(|t| &t.id == id) else {
                    // Removed in the editor, edited in the file?
                    if bv != fv {
                        ctx.conflicts.push(Conflict {
                            id: id.clone(),
                            field: "track".into(),
                            file_value: None,
                            editor_value: None,
                            kept: prefer,
                            message: format!("track {id} was edited in the file but removed in the editor"),
                        });
                        if prefer == Prefer::File {
                            m.project.tracks.push(Track {
                                id: id.clone(),
                                name: ft.name.clone(),
                                order: 0,
                                kind: ft.kind.clone(),
                                fields: ft.fields.clone(),
                            });
                        }
                    }
                    continue;
                };
                let cv = track_view(&m.project.tracks[ci]);
                let keys: BTreeSet<&String> = bv.keys().chain(fv.keys()).collect();
                for k in keys {
                    if bv.get(k) != fv.get(k)
                        && let Some(new) = ctx.decide(id, k, bv.get(k).cloned(), fv.get(k).cloned(), cv.get(k).cloned())
                    {
                        set_track_field(&mut m.project.tracks[ci], k, new);
                    }
                }
            }
            None => {
                if m.track(id).is_none() {
                    m.project.tracks.push(Track {
                        id: id.clone(),
                        name: ft.name.clone(),
                        order: 0,
                        kind: ft.kind.clone(),
                        fields: ft.fields.clone(),
                    });
                }
            }
        }
    }
    for bt in &base.project.tracks {
        if file_track_ids.contains(&bt.id) {
            continue;
        }
        if let Some(ci) = m.project.tracks.iter().position(|t| t.id == bt.id) {
            let changed = track_view(&m.project.tracks[ci]) != track_view(bt);
            if changed {
                ctx.conflicts.push(Conflict {
                    id: bt.id.clone(),
                    field: "track".into(),
                    file_value: None,
                    editor_value: None,
                    kept: prefer,
                    message: format!("track {} was removed in the file but changed in the editor", bt.id),
                });
            }
            if !changed || prefer == Prefer::File {
                m.project.tracks.remove(ci);
            }
        }
    }
    // Order.
    let present: BTreeSet<String> = m.project.tracks.iter().map(|t| t.id.clone()).collect();
    let b_order: Vec<String> = base.project.tracks.iter().map(|t| t.id.clone()).collect();
    let c_order: Vec<String> = current.project.tracks.iter().map(|t| t.id.clone()).collect();
    let common_bf: Vec<&String> = b_order.iter().filter(|i| file_track_ids.contains(i)).collect();
    let f_common: Vec<&String> = file_track_ids.iter().filter(|i| b_order.contains(i)).collect();
    let common_bc: Vec<&String> = b_order.iter().filter(|i| c_order.contains(i)).collect();
    let c_common: Vec<&String> = c_order.iter().filter(|i| b_order.contains(i)).collect();
    let file_changed = common_bf != f_common;
    let editor_changed = common_bc != c_common;
    if file_changed
        && editor_changed
        && f_common != c_common.iter().filter(|i| file_track_ids.contains(i)).copied().collect::<Vec<_>>()
    {
        ctx.conflicts.push(Conflict {
            id: "timeline".into(),
            field: "track order".into(),
            file_value: Some(file_track_ids.clone().into()),
            editor_value: Some(c_order.clone().into()),
            kept: prefer,
            message: "the track order was changed both in the file and in the editor".into(),
        });
    }
    let mut order: Vec<String>;
    if file_changed && (!editor_changed || prefer == Prefer::File) {
        order = file_track_ids.iter().filter(|i| present.contains(*i)).cloned().collect();
        let missing: Vec<String> =
            c_order.iter().filter(|i| present.contains(*i) && !order.contains(i)).cloned().collect();
        insert_after_predecessors(&mut order, &c_order, &missing);
    } else {
        order = c_order.iter().filter(|i| present.contains(*i)).cloned().collect();
        let missing: Vec<String> =
            file_track_ids.iter().filter(|i| present.contains(*i) && !order.contains(i)).cloned().collect();
        insert_after_predecessors(&mut order, &file_track_ids, &missing);
    }
    for id in &present {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    let mut tracks = std::mem::take(&mut m.project.tracks);
    for (i, id) in order.iter().enumerate() {
        let at = tracks.iter().position(|t| &t.id == id).unwrap();
        let mut t = tracks.swap_remove(at);
        t.order = i as u32;
        m.project.tracks.push(t);
    }

    // Items.
    let taken_item = |id: &str| {
        base.project.items.contains_key(id)
            || current.project.items.contains_key(id)
            || file.items.iter().any(|i| i.id.as_deref() == Some(id))
    };
    let mut seen = BTreeSet::new();
    let mut new_ids = Vec::new();
    let mut claimed_existing: BTreeSet<String> = BTreeSet::new();
    for fi in &file.items {
        let track_id = &file_track_ids[fi.track];
        let fv = file_item_view(fi, track_id);
        match fi.id.as_ref().filter(|id| base.project.items.contains_key(*id)) {
            Some(id) => {
                seen.insert(id.clone());
                positions.insert(id.clone(), fi.pos);
                let bi = &base.project.items[id];
                let bv = item_view(bi);
                let ch = changes(&bi.kind, &bv, &fv);
                if ch.is_empty() {
                    continue;
                }
                match m.project.items.get(id).cloned() {
                    None => {
                        ctx.conflicts.push(Conflict {
                            id: id.clone(),
                            field: "item".into(),
                            file_value: Some(Value::Object(fv.clone())),
                            editor_value: None,
                            kept: prefer,
                            message: format!("item {id} was edited in the file but removed in the editor"),
                        });
                        if prefer == Prefer::File {
                            let item = Item {
                                id: id.clone(),
                                track_id: track_id.clone(),
                                kind: fi.kind.clone(),
                                start: fi.start,
                                end: fi.end,
                                fields: fi.fields.clone(),
                            };
                            m.project.items.insert(id.clone(), Arc::new(item));
                        }
                    }
                    Some(ci) => {
                        let cv = item_view(&ci);
                        let mut item = (*ci).clone();
                        for (p, f) in ch {
                            if let Some(new) = ctx.decide(id, &path_str(&p), get(&bv, &p), f, get(&cv, &p)) {
                                set_path(&mut item, &p, new);
                            }
                        }
                        if item != *ci {
                            m.project.items.insert(id.clone(), Arc::new(item));
                        }
                    }
                }
            }
            None => {
                // A new item. If the editor already holds an identical item added since the base (the same file
                // applied twice), it is already applied.
                let same = current.project.items.iter().find(|(cid, ci)| {
                    !base.project.items.contains_key(*cid) && !claimed_existing.contains(*cid) && item_view(ci) == fv
                });
                if let Some((cid, _)) = same {
                    claimed_existing.insert(cid.clone());
                    positions.insert(cid.clone(), fi.pos);
                    continue;
                }
                let id = match &fi.id {
                    Some(id) if !current.project.items.contains_key(id) && !new_ids.contains(id) => id.clone(),
                    _ => fresh_id("i_", &|x: &str| taken_item(x) || new_ids.iter().any(|n| n == x), &mut seed),
                };
                positions.insert(id.clone(), fi.pos);
                new_ids.push(id.clone());
                let item = Item {
                    id: id.clone(),
                    track_id: track_id.clone(),
                    kind: fi.kind.clone(),
                    start: fi.start,
                    end: fi.end,
                    fields: fi.fields.clone(),
                };
                m.project.items.insert(id, Arc::new(item));
            }
        }
    }
    let mut removed = Vec::new();
    for (id, bi) in &base.project.items {
        if seen.contains(id) {
            continue;
        }
        if let Some(ci) = current.project.items.get(id) {
            let changed = item_view(ci) != item_view(bi);
            if changed {
                ctx.conflicts.push(Conflict {
                    id: id.clone(),
                    field: "item".into(),
                    file_value: None,
                    editor_value: Some(ci.to_json()),
                    kept: prefer,
                    message: format!("item {id} was removed in the file but changed in the editor"),
                });
            }
            if !changed || prefer == Prefer::File {
                m.project.items.remove(id);
                removed.push(Removed { id: id.clone(), name: ci.name().to_string() });
            }
        }
    }
    Merged { target: m, conflicts: ctx.conflicts, removed, positions, new_ids }
}
