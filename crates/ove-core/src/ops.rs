//! Document operations and atomic batch apply (design D6).
//!
//! The ten timeline op kinds from the reference contract, plus four project/media ops that the reference performs
//! outside its op list but which must go through the one document pipeline here (media registration, probe status,
//! project name and metadata). Every op captures before-values so the batch's inverse can be computed.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::invariants::{self, item_path, track_path};
use crate::model::{Document, Fields, Item, MediaItem, RegistryEntry, Track};
use crate::schema::{Issue, Severity};
use crate::time::json_to_micros;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Insert a track at position `track.order` (clamped); later tracks move down.
    AddTrack {
        track: Value,
    },
    /// Set track fields (not `id` or `order`).
    UpdateTrack {
        id: String,
        set: Map<String, Value>,
    },
    /// Remove a track; later tracks move up.
    RemoveTrack {
        id: String,
    },
    /// New track order, top first: a permutation of all track ids.
    ReorderTracks {
        order: Vec<String>,
    },
    /// Remove optional track fields.
    UnsetTrackFields {
        id: String,
        fields: Vec<String>,
    },
    AddItem {
        item: Value,
    },
    /// Set item fields (head fields except `id` included).
    UpdateItem {
        id: String,
        set: Map<String, Value>,
    },
    RemoveItems {
        ids: Vec<String>,
    },
    /// Remove optional item fields.
    UnsetItemFields {
        id: String,
        fields: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    SetAspectRatio {
        aspect_ratio: String,
    },
    /// Register a media item and optionally its registry entry.
    AddMedia {
        media: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<Value>,
    },
    /// Change media item fields and/or registry entry fields.
    #[serde(rename_all = "camelCase")]
    UpdateMedia {
        id: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        set: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        unset: Vec<String>,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        entry_set: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entry_unset: Vec<String>,
    },
    /// Remove a media item and its registry entry.
    RemoveMedia {
        id: String,
    },
    /// Set or unset project-level fields (name and metadata such as cast); not ids, revisions or structure.
    UpdateProject {
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        set: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        unset: Vec<String>,
    },
}

impl Op {
    pub fn kind(&self) -> &'static str {
        match self {
            Op::AddTrack { .. } => "addTrack",
            Op::UpdateTrack { .. } => "updateTrack",
            Op::RemoveTrack { .. } => "removeTrack",
            Op::ReorderTracks { .. } => "reorderTracks",
            Op::UnsetTrackFields { .. } => "unsetTrackFields",
            Op::AddItem { .. } => "addItem",
            Op::UpdateItem { .. } => "updateItem",
            Op::RemoveItems { .. } => "removeItems",
            Op::UnsetItemFields { .. } => "unsetItemFields",
            Op::SetAspectRatio { .. } => "setAspectRatio",
            Op::AddMedia { .. } => "addMedia",
            Op::UpdateMedia { .. } => "updateMedia",
            Op::RemoveMedia { .. } => "removeMedia",
            Op::UpdateProject { .. } => "updateProject",
        }
    }
}

/// A batch rejection: the op index (when attributable), the violated invariant or op rule, and a message.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}{}: {}", .op_index.map(|i| format!("op {i}: ")).unwrap_or_default(), .code, .message)]
pub struct BatchError {
    pub op_index: Option<usize>,
    pub code: &'static str,
    pub path: String,
    pub message: String,
    /// Every error found (the first is repeated in the fields above).
    pub all: Vec<Issue>,
}

impl BatchError {
    pub fn op(index: usize, code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        let path = path.into();
        let message = message.into();
        BatchError {
            op_index: Some(index),
            code,
            path: path.clone(),
            message: message.clone(),
            all: vec![Issue { severity: Severity::Error, code, path, message }],
        }
    }
}

/// Result of a successful batch.
#[derive(Debug, Clone)]
pub struct Applied {
    pub doc: Document,
    /// Ops that undo the batch when applied to `doc`.
    pub inverse: Vec<Op>,
    /// Warnings (for example unknown keys in known objects).
    pub warnings: Vec<Issue>,
    pub touched_items: BTreeSet<String>,
    pub touched_tracks: BTreeSet<String>,
}

/// Applies a batch to a copy of `doc`, all or nothing. Invariants are checked after the last op.
pub fn apply_batch(doc: &Document, ops: &[Op]) -> Result<Applied, BatchError> {
    let mut next = doc.clone();
    let mut inverse_chunks: Vec<Vec<Op>> = Vec::with_capacity(ops.len());
    let mut touched_items = BTreeSet::new();
    let mut touched_tracks = BTreeSet::new();
    // Last op index that touched each entity, for attributing end-of-batch invariant errors.
    let mut last_touch: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut last_global = None;

    for (i, op) in ops.iter().enumerate() {
        let inv = apply_op(&mut next, op, i, &mut |e: Touch| {
            match e {
                Touch::Item(id) => {
                    last_touch.insert(item_path(&id), i);
                    touched_items.insert(id);
                }
                Touch::Track(id) => {
                    last_touch.insert(track_path(&id), i);
                    touched_tracks.insert(id);
                }
                Touch::Global => {}
            }
            last_global = Some(i);
        })?;
        inverse_chunks.push(inv);
    }

    let references_may_break = ops.iter().any(|o| matches!(o, Op::RemoveTrack { .. } | Op::RemoveMedia { .. }));
    let issues = invariants::check_touched(&next, &touched_tracks, &touched_items, references_may_break);
    let (errs, warnings): (Vec<Issue>, Vec<Issue>) = issues.into_iter().partition(|i| i.severity == Severity::Error);
    if let Some(first) = errs.first() {
        let key = first.path.split('.').next().unwrap_or_default().to_string();
        let op_index = last_touch.get(&key).copied().or(last_global);
        return Err(BatchError {
            op_index,
            code: first.code,
            path: first.path.clone(),
            message: first.message.clone(),
            all: errs,
        });
    }
    let inverse = inverse_chunks.into_iter().rev().flatten().collect();
    Ok(Applied { doc: next, inverse, warnings, touched_items, touched_tracks })
}

enum Touch {
    Item(String),
    Track(String),
    Global,
}

fn renumber(tracks: &mut [Track]) {
    for (i, t) in tracks.iter_mut().enumerate() {
        t.order = i as u32;
    }
}

fn track_idx(doc: &Document, id: &str, i: usize) -> Result<usize, BatchError> {
    doc.project
        .tracks
        .iter()
        .position(|t| t.id == id)
        .ok_or_else(|| BatchError::op(i, "op.target.missing", track_path(id), format!("track {id} does not exist")))
}

/// Applies one op in place and returns its inverse ops (in application order).
fn apply_op(doc: &mut Document, op: &Op, i: usize, touch: &mut dyn FnMut(Touch)) -> Result<Vec<Op>, BatchError> {
    let malformed = |path: String, e: crate::model::ReadError| BatchError::op(i, e.code, path, e.to_string());
    match op {
        Op::AddTrack { track } => {
            let t = Track::from_json(track, "track").map_err(|e| malformed("track".into(), e))?;
            if doc.track(&t.id).is_some() {
                return Err(BatchError::op(
                    i,
                    "track.id.unique",
                    track_path(&t.id),
                    format!("track {} already exists", t.id),
                ));
            }
            let id = t.id.clone();
            let at = (t.order as usize).min(doc.project.tracks.len());
            doc.project.tracks.insert(at, t);
            renumber(&mut doc.project.tracks);
            touch(Touch::Track(id.clone()));
            Ok(vec![Op::RemoveTrack { id }])
        }
        Op::RemoveTrack { id } => {
            let at = track_idx(doc, id, i)?;
            let t = doc.project.tracks.remove(at);
            renumber(&mut doc.project.tracks);
            touch(Touch::Track(id.clone()));
            Ok(vec![Op::AddTrack { track: t.to_json() }])
        }
        Op::UpdateTrack { id, set } => {
            let at = track_idx(doc, id, i)?;
            let t = &mut doc.project.tracks[at];
            let mut prev = Map::new();
            let mut absent = Vec::new();
            for (k, v) in set {
                match k.as_str() {
                    "id" | "order" => {
                        return Err(BatchError::op(
                            i,
                            "op.field.immutable",
                            format!("{}.{k}", track_path(id)),
                            format!("{k} cannot be set with updateTrack"),
                        ));
                    }
                    "name" | "type" => {
                        let s = v.as_str().ok_or_else(|| {
                            BatchError::op(
                                i,
                                "value.type",
                                format!("{}.{k}", track_path(id)),
                                format!("{k} must be a string"),
                            )
                        })?;
                        prev.insert(k.clone(), t.get(k).unwrap());
                        if k == "name" { t.name = s.into() } else { t.kind = s.into() }
                    }
                    _ => {
                        match t.fields.insert(k.clone(), v.clone()) {
                            Some(old) => prev.insert(k.clone(), old),
                            None => {
                                absent.push(k.clone());
                                None
                            }
                        };
                    }
                }
            }
            touch(Touch::Track(id.clone()));
            Ok(update_inverse(
                |set| Op::UpdateTrack { id: id.clone(), set },
                |fields| Op::UnsetTrackFields { id: id.clone(), fields },
                prev,
                absent,
            ))
        }
        Op::UnsetTrackFields { id, fields } => {
            let at = track_idx(doc, id, i)?;
            let t = &mut doc.project.tracks[at];
            let mut prev = Map::new();
            for k in fields {
                if Track::HEAD.contains(&k.as_str()) {
                    return Err(BatchError::op(
                        i,
                        "op.field.required",
                        format!("{}.{k}", track_path(id)),
                        format!("{k} is required and cannot be unset"),
                    ));
                }
                if let Some(old) = t.fields.shift_remove(k) {
                    prev.insert(k.clone(), old);
                }
            }
            touch(Touch::Track(id.clone()));
            Ok(if prev.is_empty() { vec![] } else { vec![Op::UpdateTrack { id: id.clone(), set: prev }] })
        }
        Op::ReorderTracks { order } => {
            let old: Vec<String> = doc.project.tracks.iter().map(|t| t.id.clone()).collect();
            let a: BTreeSet<&String> = old.iter().collect();
            let b: BTreeSet<&String> = order.iter().collect();
            if a != b || order.len() != old.len() {
                return Err(BatchError::op(
                    i,
                    "op.reorder.permutation",
                    "tracks",
                    "reorderTracks must list every track id exactly once",
                ));
            }
            let mut tracks = std::mem::take(&mut doc.project.tracks);
            let mut new = Vec::with_capacity(tracks.len());
            for id in order {
                let at = tracks.iter().position(|t| &t.id == id).unwrap();
                new.push(tracks.swap_remove(at));
            }
            doc.project.tracks = new;
            renumber(&mut doc.project.tracks);
            touch(Touch::Global);
            Ok(vec![Op::ReorderTracks { order: old }])
        }
        Op::AddItem { item } => {
            let it = Item::from_json(item, "item").map_err(|e| malformed("item".into(), e))?;
            if doc.project.items.contains_key(&it.id) {
                return Err(BatchError::op(
                    i,
                    "item.id.unique",
                    item_path(&it.id),
                    format!("item {} already exists", it.id),
                ));
            }
            let id = it.id.clone();
            doc.project.items.insert(id.clone(), Arc::new(it));
            touch(Touch::Item(id.clone()));
            Ok(vec![Op::RemoveItems { ids: vec![id] }])
        }
        Op::RemoveItems { ids } => {
            let mut inv = Vec::with_capacity(ids.len());
            for id in ids {
                let it = doc.project.items.remove(id).ok_or_else(|| {
                    BatchError::op(i, "op.target.missing", item_path(id), format!("item {id} does not exist"))
                })?;
                touch(Touch::Item(id.clone()));
                inv.push(Op::AddItem { item: it.to_json() });
            }
            inv.reverse();
            Ok(inv)
        }
        Op::UpdateItem { id, set } => {
            let arc = doc.project.items.get_mut(id).ok_or_else(|| {
                BatchError::op(i, "op.target.missing", item_path(id), format!("item {id} does not exist"))
            })?;
            let it = Arc::make_mut(arc);
            let mut prev = Map::new();
            let mut absent = Vec::new();
            for (k, v) in set {
                let path = format!("{}.{k}", item_path(id));
                match k.as_str() {
                    "id" => return Err(BatchError::op(i, "op.field.immutable", path, "id cannot be changed")),
                    "trackId" | "type" => {
                        let s = v
                            .as_str()
                            .ok_or_else(|| BatchError::op(i, "value.type", path, format!("{k} must be a string")))?;
                        prev.insert(k.clone(), it.get(k).unwrap());
                        if k == "trackId" { it.track_id = s.into() } else { it.kind = s.into() }
                    }
                    "startTime" | "endTime" => {
                        let us = json_to_micros(v).ok_or_else(|| {
                            BatchError::op(i, "value.type", path, format!("{k} must be a finite number of seconds"))
                        })?;
                        prev.insert(k.clone(), it.get(k).unwrap());
                        if k == "startTime" { it.start = us } else { it.end = us }
                    }
                    _ => match it.fields.insert(k.clone(), v.clone()) {
                        Some(old) => {
                            prev.insert(k.clone(), old);
                        }
                        None => absent.push(k.clone()),
                    },
                }
            }
            let kind = it.kind.clone();
            crate::model::normalise_item_fields(&kind, &mut it.fields);
            touch(Touch::Item(id.clone()));
            Ok(update_inverse(
                |set| Op::UpdateItem { id: id.clone(), set },
                |fields| Op::UnsetItemFields { id: id.clone(), fields },
                prev,
                absent,
            ))
        }
        Op::UnsetItemFields { id, fields } => {
            let arc = doc.project.items.get_mut(id).ok_or_else(|| {
                BatchError::op(i, "op.target.missing", item_path(id), format!("item {id} does not exist"))
            })?;
            let it = Arc::make_mut(arc);
            let mut prev = Map::new();
            for k in fields {
                if Item::HEAD.contains(&k.as_str()) {
                    return Err(BatchError::op(
                        i,
                        "op.field.required",
                        format!("{}.{k}", item_path(id)),
                        format!("{k} is required and cannot be unset"),
                    ));
                }
                if let Some(old) = it.fields.shift_remove(k) {
                    prev.insert(k.clone(), old);
                }
            }
            touch(Touch::Item(id.clone()));
            Ok(if prev.is_empty() { vec![] } else { vec![Op::UpdateItem { id: id.clone(), set: prev }] })
        }
        Op::SetAspectRatio { aspect_ratio } => {
            let old = std::mem::replace(&mut doc.project.aspect_ratio, aspect_ratio.clone());
            touch(Touch::Global);
            Ok(vec![Op::SetAspectRatio { aspect_ratio: old }])
        }
        Op::AddMedia { media, entry } => {
            let m = MediaItem::from_json(media, "media").map_err(|e| malformed("media".into(), e))?;
            if doc.project.media.contains_key(&m.id) {
                return Err(BatchError::op(
                    i,
                    "media.id.unique",
                    format!("media[{}]", m.id),
                    format!("media {} already exists", m.id),
                ));
            }
            if let Some(e) = entry {
                let e = RegistryEntry::from_json(e, "entry").map_err(|e| malformed("entry".into(), e))?;
                if e.media_id != m.id {
                    return Err(BatchError::op(
                        i,
                        "op.malformed",
                        "entry.mediaId",
                        "entry.mediaId must equal media.id",
                    ));
                }
                doc.registry.insert(e.media_id.clone(), e);
            }
            let id = m.id.clone();
            doc.project.media.insert(id.clone(), Arc::new(m));
            touch(Touch::Global);
            Ok(vec![Op::RemoveMedia { id }])
        }
        Op::RemoveMedia { id } => {
            let m = doc.project.media.remove(id).ok_or_else(|| {
                BatchError::op(i, "op.target.missing", format!("media[{id}]"), format!("media {id} does not exist"))
            })?;
            let entry = doc.registry.remove(id).map(|e| e.to_json());
            touch(Touch::Global);
            Ok(vec![Op::AddMedia { media: m.to_json(), entry }])
        }
        Op::UpdateMedia { id, set, unset, entry_set, entry_unset } => {
            let arc = doc.project.media.get_mut(id).ok_or_else(|| {
                BatchError::op(i, "op.target.missing", format!("media[{id}]"), format!("media {id} does not exist"))
            })?;
            let m = Arc::make_mut(arc);
            let (mut prev, mut absent) = (Map::new(), Vec::new());
            for (k, v) in set {
                if k == "id" {
                    return Err(BatchError::op(
                        i,
                        "op.field.immutable",
                        format!("media[{id}].id"),
                        "id cannot be changed",
                    ));
                }
                match m.fields.insert(k.clone(), v.clone()) {
                    Some(old) => {
                        prev.insert(k.clone(), old);
                    }
                    None => absent.push(k.clone()),
                }
            }
            for k in unset {
                if let Some(old) = m.fields.shift_remove(k) {
                    prev.insert(k.clone(), old);
                }
            }
            let (mut eprev, mut eabsent) = (Map::new(), Vec::new());
            if !entry_set.is_empty() || !entry_unset.is_empty() {
                let e = doc.registry.get_mut(id).ok_or_else(|| {
                    BatchError::op(
                        i,
                        "op.target.missing",
                        format!("media[{id}]"),
                        format!("media {id} has no registry entry"),
                    )
                })?;
                for (k, v) in entry_set {
                    match k.as_str() {
                        "mediaId" => {
                            return Err(BatchError::op(
                                i,
                                "op.field.immutable",
                                format!("media[{id}].mediaId"),
                                "mediaId cannot be changed",
                            ));
                        }
                        "nativePath" => {
                            let s = v.as_str().ok_or_else(|| {
                                BatchError::op(
                                    i,
                                    "value.type",
                                    format!("media[{id}].nativePath"),
                                    "nativePath must be a string",
                                )
                            })?;
                            eprev.insert(k.clone(), e.native_path.clone().into());
                            e.native_path = s.into();
                        }
                        "ownedByBundle" => {
                            let b = v.as_bool().ok_or_else(|| {
                                BatchError::op(
                                    i,
                                    "value.type",
                                    format!("media[{id}].ownedByBundle"),
                                    "ownedByBundle must be true or false",
                                )
                            })?;
                            eprev.insert(k.clone(), e.owned_by_bundle.into());
                            e.owned_by_bundle = b;
                        }
                        _ => match e.fields.insert(k.clone(), v.clone()) {
                            Some(old) => {
                                eprev.insert(k.clone(), old);
                            }
                            None => eabsent.push(k.clone()),
                        },
                    }
                }
                for k in entry_unset {
                    if RegistryEntry::HEAD.contains(&k.as_str()) {
                        return Err(BatchError::op(
                            i,
                            "op.field.required",
                            format!("media[{id}].{k}"),
                            format!("{k} is required"),
                        ));
                    }
                    if let Some(old) = e.fields.shift_remove(k) {
                        eprev.insert(k.clone(), old);
                    }
                }
            }
            touch(Touch::Global);
            Ok(vec![Op::UpdateMedia {
                id: id.clone(),
                set: prev,
                unset: absent,
                entry_set: eprev,
                entry_unset: eabsent,
            }])
        }
        Op::UpdateProject { set, unset } => {
            const LOCKED: [&str; 8] =
                ["id", "createdAt", "updatedAt", "stateUpdatedAt", "aspectRatio", "media", "timelineItems", "tracks"];
            let p = &mut doc.project;
            let (mut prev, mut absent) = (Map::new(), Vec::new());
            for (k, v) in set {
                if LOCKED.contains(&k.as_str()) {
                    return Err(BatchError::op(
                        i,
                        "op.field.immutable",
                        format!("project.{k}"),
                        format!("{k} cannot be set with updateProject"),
                    ));
                }
                if k == "name" {
                    let s = v
                        .as_str()
                        .ok_or_else(|| BatchError::op(i, "value.type", "project.name", "name must be a string"))?;
                    prev.insert(k.clone(), p.name.clone().into());
                    p.name = s.into();
                    continue;
                }
                match p.extra.insert(k.clone(), v.clone()) {
                    Some(old) => {
                        prev.insert(k.clone(), old);
                    }
                    None => absent.push(k.clone()),
                }
            }
            for k in unset {
                if LOCKED.contains(&k.as_str()) || k == "name" {
                    return Err(BatchError::op(
                        i,
                        "op.field.required",
                        format!("project.{k}"),
                        format!("{k} is required"),
                    ));
                }
                if let Some(old) = p.extra.shift_remove(k) {
                    prev.insert(k.clone(), old);
                }
            }
            touch(Touch::Global);
            Ok(vec![Op::UpdateProject { set: prev, unset: absent }])
        }
    }
}

fn update_inverse(
    set_op: impl FnOnce(Map<String, Value>) -> Op,
    unset_op: impl FnOnce(Vec<String>) -> Op,
    prev: Map<String, Value>,
    absent: Vec<String>,
) -> Vec<Op> {
    let mut v = Vec::new();
    if !absent.is_empty() {
        v.push(unset_op(absent));
    }
    if !prev.is_empty() {
        v.push(set_op(prev));
    }
    v
}

/// Short human summary of a batch, e.g. `2 items added, 1 item updated`.
pub fn summarise(ops: &[Op]) -> String {
    let mut counts: indexmap::IndexMap<&str, usize> = indexmap::IndexMap::new();
    for op in ops {
        let (label, n) = match op {
            Op::AddItem { .. } => ("item added", 1),
            Op::UpdateItem { .. } | Op::UnsetItemFields { .. } => ("item update", 1),
            Op::RemoveItems { ids } => ("item removed", ids.len()),
            Op::AddTrack { .. } => ("track added", 1),
            Op::RemoveTrack { .. } => ("track removed", 1),
            Op::UpdateTrack { .. } | Op::UnsetTrackFields { .. } | Op::ReorderTracks { .. } => ("track update", 1),
            Op::SetAspectRatio { .. } => ("aspect change", 1),
            Op::AddMedia { .. } => ("media added", 1),
            Op::RemoveMedia { .. } => ("media removed", 1),
            Op::UpdateMedia { .. } => ("media update", 1),
            Op::UpdateProject { .. } => ("project update", 1),
        };
        *counts.entry(label).or_default() += n;
    }
    if counts.is_empty() {
        return "no changes".into();
    }
    counts
        .iter()
        .map(|(l, n)| if *n == 1 { format!("1 {l}") } else { format!("{n} {l}s") })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Field map type re-exported for op construction.
pub type SetMap = Fields;
