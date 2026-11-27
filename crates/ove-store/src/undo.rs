//! Selective undo (unify-agent-tool-layer D4): revert chosen revisions — one actor's, one turn's — as one new
//! revision, keeping every other change. The inverse operations of the chosen revisions are composed newest first and
//! applied to the document as it is now. When a later revision outside the set changed the same item fields, nothing
//! changes and the conflicting items are listed.

use std::collections::{BTreeMap, BTreeSet};

use ove_core::ops::Op;
use ove_core::revision::{Actor, RevisionRecord, Source};
use serde::Serialize;
use serde_json::Value;

use crate::error::StoreError;
use crate::project::{Committed, Project, read_record};

/// What a batch touches: (object, field), where object is `item:<id>`, `track:<id>`, `media:<id>` or `project`, and
/// field `*` means the whole object (added or removed).
pub fn touched(ops: &[Op]) -> BTreeSet<(String, String)> {
    let mut t = BTreeSet::new();
    let id_of = |v: &Value| v.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    for op in ops {
        match op {
            Op::AddTrack { track } => {
                t.insert((format!("track:{}", id_of(track)), "*".into()));
            }
            Op::RemoveTrack { id } => {
                t.insert((format!("track:{id}"), "*".into()));
            }
            Op::UpdateTrack { id, set } => t.extend(set.keys().map(|k| (format!("track:{id}"), k.clone()))),
            Op::UnsetTrackFields { id, fields } => t.extend(fields.iter().map(|k| (format!("track:{id}"), k.clone()))),
            Op::ReorderTracks { .. } => {
                t.insert(("project".into(), "trackOrder".into()));
            }
            Op::AddItem { item } => {
                t.insert((format!("item:{}", id_of(item)), "*".into()));
            }
            Op::RemoveItems { ids } => t.extend(ids.iter().map(|id| (format!("item:{id}"), "*".into()))),
            Op::UpdateItem { id, set } => t.extend(set.keys().map(|k| (format!("item:{id}"), k.clone()))),
            Op::UnsetItemFields { id, fields } => t.extend(fields.iter().map(|k| (format!("item:{id}"), k.clone()))),
            Op::SetAspectRatio { .. } => {
                t.insert(("project".into(), "aspectRatio".into()));
            }
            Op::AddMedia { media, .. } => {
                t.insert((format!("media:{}", id_of(media)), "*".into()));
            }
            Op::RemoveMedia { id } => {
                t.insert((format!("media:{id}"), "*".into()));
            }
            Op::UpdateMedia { id, set, unset, entry_set, entry_unset } => {
                for k in set.keys().chain(unset).chain(entry_set.keys()).chain(entry_unset) {
                    t.insert((format!("media:{id}"), k.clone()));
                }
            }
            Op::UpdateProject { set, unset } => {
                t.extend(set.keys().chain(unset).map(|k| ("project".into(), k.clone())))
            }
        }
    }
    t
}

/// A later change that an undo would overwrite.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoConflict {
    /// `item:<id>`, `track:<id>`, `media:<id>` or `project`.
    pub object: String,
    /// The item id, when the object is an item.
    pub item: Option<String>,
    pub field: String,
    /// The later revision that changed it.
    pub rev: i64,
    pub summary: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("nothing to undo")]
    Nothing,
    #[error("later changes touched the same items")]
    Conflict(Vec<UndoConflict>),
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn overlaps(a: &(String, String), b: &(String, String)) -> bool {
    a.0 == b.0 && (a.1 == b.1 || a.1 == "*" || b.1 == "*")
}

/// Later revisions (outside `chosen`) that changed what the chosen revisions changed. The app's own bookkeeping
/// (source `migration`, such as filling in the content ids of new media) does not conflict with adding or removing a
/// whole object: undoing that addition removes the derived fields with it, and nobody's work is lost.
pub fn conflicts(chosen: &[RevisionRecord], later: &[RevisionRecord]) -> Vec<UndoConflict> {
    let chosen_revs: BTreeSet<i64> = chosen.iter().map(|r| r.rev).collect();
    let mut found: BTreeMap<(String, String), UndoConflict> = BTreeMap::new();
    for c in chosen {
        let mine = touched(&c.ops);
        for l in later.iter().filter(|l| l.rev > c.rev && !chosen_revs.contains(&l.rev)) {
            let bookkeeping = l.source == Source::Migration;
            for t in touched(&l.ops) {
                if let Some(m) = mine.iter().find(|m| overlaps(m, &t) && !(bookkeeping && m.1 == "*")) {
                    let field = if m.1 == "*" { t.1.clone() } else { m.1.clone() };
                    found.entry((t.0.clone(), field.clone())).or_insert_with(|| UndoConflict {
                        item: t.0.strip_prefix("item:").map(str::to_string),
                        object: t.0.clone(),
                        field,
                        rev: l.rev,
                        summary: l.summary.clone(),
                    });
                }
            }
        }
    }
    found.into_values().collect()
}

impl Project {
    /// Reverts the revisions `revs` as one new revision (recorded with `source`, `summary` and `actor`), keeping
    /// every other change. Refused, changing nothing, when a later revision outside `revs` changed the same fields.
    pub fn undo_selected(
        &mut self,
        revs: &[i64],
        source: Source,
        summary: String,
        actor: Option<Actor>,
    ) -> Result<Committed, UndoError> {
        let mut sorted = revs.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let Some(&first) = sorted.first() else { return Err(UndoError::Nothing) };
        let chosen: Vec<RevisionRecord> = sorted
            .iter()
            .map(|r| {
                read_record(self.bundle(), *r).map_err(|_| {
                    StoreError::refused(
                        "undo.unavailable",
                        format!("revision {r} is no longer in the project's history, so it can't be undone this way"),
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        let later: Vec<RevisionRecord> = self.records()?.into_iter().filter(|r| r.rev > first).collect();
        let found = conflicts(&chosen, &later);
        if !found.is_empty() {
            return Err(UndoError::Conflict(found));
        }
        let ops: Vec<Op> = chosen.iter().rev().flat_map(|r| r.inverse.clone()).collect();
        if ops.is_empty() {
            return Err(UndoError::Nothing);
        }
        Ok(self.apply_as(ops, source, Some(summary), actor)?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    fn project() -> (tempfile::TempDir, Project) {
        let d = tempfile::tempdir().unwrap();
        let mut p = Project::create(&d.path().join("p.ove"), "p_1", "P", Arc::new(ove_core::SystemClock)).unwrap();
        let ops: Vec<Op> = serde_json::from_value(json!([
            {"op": "addTrack", "track": {"id": "t", "name": "T", "order": 0, "type": "video"}},
            {"op": "addItem", "item": {"id": "a", "trackId": "t", "type": "text", "startTime": 0, "endTime": 1}},
            {"op": "addItem", "item": {"id": "b", "trackId": "t", "type": "text", "startTime": 1, "endTime": 2}}
        ]))
        .unwrap();
        p.apply(ops, Source::Gui, None).unwrap();
        (d, p)
    }

    fn set(p: &mut Project, id: &str, v: serde_json::Value, source: Source) -> i64 {
        let set = v.as_object().unwrap().clone();
        p.apply(vec![Op::UpdateItem { id: id.into(), set }], source, None).unwrap().record.rev
    }

    #[test]
    fn only_the_chosen_revisions_are_reverted() {
        let (_d, mut p) = project();
        set(&mut p, "a", json!({"name": "person 1"}), Source::Gui);
        let agent = set(&mut p, "b", json!({"name": "agent"}), Source::Mcp);
        set(&mut p, "a", json!({"textContent": "person 2"}), Source::Gui);
        let c = p.undo_selected(&[agent], Source::Undo, "undo".into(), None).unwrap();
        assert_eq!(c.record.source, Source::Undo);
        let d = p.doc();
        assert_eq!(d.item("b").unwrap().get("name"), None);
        assert_eq!(d.item("a").unwrap().get("name"), Some(json!("person 1")));
        assert_eq!(d.item("a").unwrap().get("textContent"), Some(json!("person 2")));
    }

    #[test]
    fn a_later_change_to_the_same_field_refuses_and_lists_the_item() {
        let (_d, mut p) = project();
        let agent = set(&mut p, "a", json!({"startTime": 5, "endTime": 6}), Source::Mcp);
        set(&mut p, "a", json!({"startTime": 7, "endTime": 8}), Source::Gui);
        let before = p.doc().clone();
        match p.undo_selected(&[agent], Source::Undo, "undo".into(), None) {
            Err(UndoError::Conflict(c)) => {
                assert_eq!(c[0].item.as_deref(), Some("a"));
                assert!(c.iter().any(|x| x.field == "startTime"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.doc(), &before);
        // Another field of the same item is not a conflict.
        let (_d, mut p) = project();
        let agent = set(&mut p, "a", json!({"name": "agent"}), Source::Mcp);
        set(&mut p, "a", json!({"startTime": 7, "endTime": 8}), Source::Gui);
        assert!(p.undo_selected(&[agent], Source::Undo, "undo".into(), None).is_ok());
    }

    #[test]
    fn the_apps_bookkeeping_on_added_media_does_not_block_undoing_the_addition() {
        let media = |p: &mut Project, source: Source| {
            let ops: Vec<Op> = serde_json::from_value(json!([
                {"op": "addMedia", "media": {"id": "m", "createdAt": 0, "addedAt": 0, "data": {"type": "image", "fileName": "g.png"}},
                 "entry": {"mediaId": "m", "nativePath": "media/generated/g.png", "ownedByBundle": true}}
            ]))
            .unwrap();
            p.apply(ops, source, None).unwrap().record.rev
        };
        let identity = |p: &mut Project, source: Source, field: &str| {
            let mut set = serde_json::Map::new();
            set.insert(field.into(), json!("x"));
            let op = Op::UpdateMedia {
                id: "m".into(),
                set,
                unset: vec![],
                entry_set: Default::default(),
                entry_unset: vec![],
            };
            p.apply(vec![op], source, None).unwrap();
        };
        // The app fills in derived fields (source migration): the agent can still undo adding the media.
        let (_d, mut p) = project();
        let agent = media(&mut p, Source::Mcp);
        identity(&mut p, Source::Migration, "contentId");
        p.undo_selected(&[agent], Source::Undo, "undo".into(), None).unwrap();
        assert!(!p.doc().project.media.contains_key("m"));
        // A person's later change to it still conflicts.
        let (_d, mut p) = project();
        let agent = media(&mut p, Source::Mcp);
        identity(&mut p, Source::Gui, "name");
        assert!(matches!(p.undo_selected(&[agent], Source::Undo, "undo".into(), None), Err(UndoError::Conflict(_))));
    }
}
