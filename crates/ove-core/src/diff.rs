//! Document diff: the ops that turn one document into another (used by restore, and by tests as an oracle).

use serde_json::{Map, Value};

use crate::model::{Document, Fields, Item, Track};
use crate::ops::Op;

fn diff_fields(a: &Fields, b: &Fields) -> (Map<String, Value>, Vec<String>) {
    let mut set = Map::new();
    let mut unset = Vec::new();
    for (k, v) in b {
        if a.get(k) != Some(v) {
            set.insert(k.clone(), v.clone());
        }
    }
    for k in a.keys() {
        if !b.contains_key(k) {
            unset.push(k.clone());
        }
    }
    (set, unset)
}

fn item_ops(a: &Item, b: &Item, out: &mut Vec<Op>) {
    let (mut set, unset) = diff_fields(&a.fields, &b.fields);
    for k in ["trackId", "type", "startTime", "endTime"] {
        let (va, vb) = (a.get(k), b.get(k));
        if va != vb {
            set.insert(k.into(), vb.unwrap());
        }
    }
    if !unset.is_empty() {
        out.push(Op::UnsetItemFields { id: a.id.clone(), fields: unset });
    }
    if !set.is_empty() {
        out.push(Op::UpdateItem { id: a.id.clone(), set });
    }
}

fn track_ops(a: &Track, b: &Track, out: &mut Vec<Op>) {
    let (mut set, unset) = diff_fields(&a.fields, &b.fields);
    if a.name != b.name {
        set.insert("name".into(), b.name.clone().into());
    }
    if a.kind != b.kind {
        set.insert("type".into(), b.kind.clone().into());
    }
    if !unset.is_empty() {
        out.push(Op::UnsetTrackFields { id: a.id.clone(), fields: unset });
    }
    if !set.is_empty() {
        out.push(Op::UpdateTrack { id: a.id.clone(), set });
    }
}

/// Ops that transform `from` into `to` (content only: revision bookkeeping is not diffed).
pub fn diff_documents(from: &Document, to: &Document) -> Vec<Op> {
    let mut ops = Vec::new();
    let (pa, pb) = (&from.project, &to.project);

    // Project metadata.
    let (mut pset, punset) = diff_fields(&pa.extra, &pb.extra);
    if pa.name != pb.name {
        pset.insert("name".into(), pb.name.clone().into());
    }
    if !pset.is_empty() || !punset.is_empty() {
        ops.push(Op::UpdateProject { set: pset, unset: punset });
    }
    if pa.aspect_ratio != pb.aspect_ratio {
        ops.push(Op::SetAspectRatio { aspect_ratio: pb.aspect_ratio.clone() });
    }

    // Media added or changed first so items can reference them.
    for (id, mb) in &pb.media {
        match pa.media.get(id) {
            None => ops.push(Op::AddMedia { media: mb.to_json(), entry: to.registry.get(id).map(|e| e.to_json()) }),
            Some(ma) => {
                let (set, unset) = diff_fields(&ma.fields, &mb.fields);
                let (mut entry_set, mut entry_unset) = (Map::new(), Vec::new());
                match (from.registry.get(id), to.registry.get(id)) {
                    (Some(ea), Some(eb)) => {
                        let (s, u) = diff_fields(&ea.fields, &eb.fields);
                        entry_set = s;
                        entry_unset = u;
                        if ea.native_path != eb.native_path {
                            entry_set.insert("nativePath".into(), eb.native_path.clone().into());
                        }
                        if ea.owned_by_bundle != eb.owned_by_bundle {
                            entry_set.insert("ownedByBundle".into(), eb.owned_by_bundle.into());
                        }
                    }
                    (ea, eb) if ea != eb => {
                        // Entry added or removed alone: replace the media item as a whole.
                        ops.push(Op::RemoveMedia { id: id.clone() });
                        ops.push(Op::AddMedia { media: mb.to_json(), entry: eb.map(|e| e.to_json()) });
                        continue;
                    }
                    _ => {}
                }
                if !set.is_empty() || !unset.is_empty() || !entry_set.is_empty() || !entry_unset.is_empty() {
                    ops.push(Op::UpdateMedia { id: id.clone(), set, unset, entry_set, entry_unset });
                }
            }
        }
    }

    // Tracks: remove, add (at their target order), update, then reorder if needed.
    for t in &pa.tracks {
        if !pb.tracks.iter().any(|u| u.id == t.id) {
            ops.push(Op::RemoveTrack { id: t.id.clone() });
        }
    }
    for t in &pb.tracks {
        match pa.tracks.iter().find(|u| u.id == t.id) {
            None => ops.push(Op::AddTrack { track: t.to_json() }),
            Some(old) => track_ops(old, t, &mut ops),
        }
    }
    let mut after: Vec<String> =
        pa.tracks.iter().filter(|t| pb.tracks.iter().any(|u| u.id == t.id)).map(|t| t.id.clone()).collect();
    for t in &pb.tracks {
        if !pa.tracks.iter().any(|u| u.id == t.id) {
            let at = (t.order as usize).min(after.len());
            after.insert(at, t.id.clone());
        }
    }
    let target: Vec<String> = pb.tracks.iter().map(|t| t.id.clone()).collect();
    if after != target {
        ops.push(Op::ReorderTracks { order: target });
    }

    // Items.
    let removed: Vec<String> = pa.items.keys().filter(|id| !pb.items.contains_key(*id)).cloned().collect();
    if !removed.is_empty() {
        ops.push(Op::RemoveItems { ids: removed });
    }
    for (id, ib) in &pb.items {
        match pa.items.get(id) {
            None => ops.push(Op::AddItem { item: ib.to_json() }),
            Some(ia) if ia != ib => item_ops(ia, ib, &mut ops),
            _ => {}
        }
    }

    // Media removed last, after the items that used them.
    for id in pa.media.keys() {
        if !pb.media.contains_key(id) {
            ops.push(Op::RemoveMedia { id: id.clone() });
        }
    }
    ops
}
