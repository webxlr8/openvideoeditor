//! Semantic diff of two documents (add-markers-keyframe-lanes-and-versions D8): the items and markers that were added,
//! removed, moved, trimmed or otherwise changed between two versions, in words a person reads.
//!
//! Classes, per item:
//! - `added` / `removed`: present on one side only.
//! - `trimmed`: its length changed (one or both edges moved by different amounts).
//! - `moved`: same length, but a new start or a new track.
//! - `changed`: same timing and track; other fields differ (`fields` names them).
//!
//! A trimmed or moved item whose other fields also changed lists those fields too. Markers follow the same rules with
//! `at`, `dur` and no track.

use serde::Serialize;
use serde_json::Value;

use crate::model::{Document, Fields, Item, Marker};
use crate::time::Micros;

/// What happened to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Added,
    Removed,
    Moved,
    Trimmed,
    Changed,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Added => "added",
            Class::Removed => "removed",
            Class::Moved => "moved",
            Class::Trimmed => "trimmed",
            Class::Changed => "changed",
        }
    }
}

/// Where an entity sat: seconds, and the track for items.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Place {
    pub start: f64,
    pub end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
}

/// One classified change.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// `item` or `marker`.
    pub entity: &'static str,
    pub id: String,
    /// The item's or marker's name, when it has one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub class: Class,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Place>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Place>,
    /// Other fields that differ (for changed, and for moved or trimmed entities that also changed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

impl Change {
    /// One line for people: `trimmed item i_1 "Intro": 0–4 s → 0–3 s`.
    pub fn line(&self) -> String {
        let name = if self.name.is_empty() { String::new() } else { format!(" \"{}\"", self.name) };
        let place = |p: &Place| {
            let t = p.track.as_ref().map(|t| format!(" on {t}")).unwrap_or_default();
            format!("{}–{} s{t}", secs(p.start), secs(p.end))
        };
        let mut s = format!("{} {} {}{name}", self.class.as_str(), self.entity, self.id);
        match (&self.before, &self.after) {
            (Some(b), Some(a)) if b != a => s.push_str(&format!(": {} → {}", place(b), place(a))),
            (Some(b), None) => s.push_str(&format!(": was {}", place(b))),
            (None, Some(a)) => s.push_str(&format!(": {}", place(a))),
            _ => {}
        }
        if !self.fields.is_empty() {
            s.push_str(&format!(" ({})", self.fields.join(", ")));
        }
        s
    }
}

fn secs(x: f64) -> String {
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn to_s(us: Micros) -> f64 {
    us as f64 / 1e6
}

fn differing(a: &Fields, b: &Fields, skip: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> =
        a.keys().chain(b.keys()).filter(|k| !skip.contains(&k.as_str()) && a.get(*k) != b.get(*k)).cloned().collect();
    keys.sort();
    keys.dedup();
    keys
}

fn name_of(fields: &Fields) -> String {
    fields.get("name").and_then(Value::as_str).unwrap_or("").to_string()
}

fn item_place(i: &Item) -> Place {
    Place { start: to_s(i.start), end: to_s(i.end), track: Some(i.track_id.clone()) }
}

fn marker_place(m: &Marker) -> Place {
    Place { start: to_s(m.at), end: to_s(m.end()), track: None }
}

fn classify(len_a: Micros, len_b: Micros, moved: bool, fields: &[String]) -> Option<Class> {
    if len_a != len_b {
        Some(Class::Trimmed)
    } else if moved {
        Some(Class::Moved)
    } else if !fields.is_empty() {
        Some(Class::Changed)
    } else {
        None
    }
}

/// The classified changes from `from` to `to`: items in canonical order, then markers in time order.
pub fn semantic_diff(from: &Document, to: &Document) -> Vec<Change> {
    let mut out = Vec::new();
    let mut ids: Vec<&String> = from.project.items.keys().chain(to.project.items.keys()).collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        let (a, b) = (from.item(id), to.item(id));
        let change = match (a, b) {
            (Some(a), None) => Change {
                entity: "item",
                id: id.clone(),
                name: name_of(&a.fields),
                class: Class::Removed,
                before: Some(item_place(a)),
                after: None,
                fields: vec![],
            },
            (None, Some(b)) => Change {
                entity: "item",
                id: id.clone(),
                name: name_of(&b.fields),
                class: Class::Added,
                before: None,
                after: Some(item_place(b)),
                fields: vec![],
            },
            (Some(a), Some(b)) => {
                let mut fields = differing(&a.fields, &b.fields, &[]);
                if a.kind != b.kind {
                    fields.insert(0, "type".into());
                }
                let moved = a.start != b.start || a.track_id != b.track_id;
                let Some(class) = classify(a.end - a.start, b.end - b.start, moved, &fields) else { continue };
                Change {
                    entity: "item",
                    id: id.clone(),
                    name: name_of(&b.fields),
                    class,
                    before: Some(item_place(a)),
                    after: Some(item_place(b)),
                    fields,
                }
            }
            (None, None) => continue,
        };
        out.push(change);
    }
    let order = |c: &Change| {
        let d = if c.class == Class::Removed { from } else { to };
        d.item(&c.id).map(|i| (d.project.tracks.iter().position(|t| t.id == i.track_id), i.start))
    };
    out.sort_by(|x, y| order(x).cmp(&order(y)).then_with(|| x.id.cmp(&y.id)));

    let mut marks = Vec::new();
    let mut mids: Vec<&String> = from.project.markers.keys().chain(to.project.markers.keys()).collect();
    mids.sort();
    mids.dedup();
    for id in mids {
        let (a, b) = (from.marker(id), to.marker(id));
        let change = match (a, b) {
            (Some(a), None) => Change {
                entity: "marker",
                id: id.clone(),
                name: name_of(&a.fields),
                class: Class::Removed,
                before: Some(marker_place(a)),
                after: None,
                fields: vec![],
            },
            (None, Some(b)) => Change {
                entity: "marker",
                id: id.clone(),
                name: name_of(&b.fields),
                class: Class::Added,
                before: None,
                after: Some(marker_place(b)),
                fields: vec![],
            },
            (Some(a), Some(b)) => {
                let mut fields = differing(&a.fields, &b.fields, &[]);
                if a.status != b.status {
                    fields.insert(0, "status".into());
                }
                let Some(class) = classify(a.dur, b.dur, a.at != b.at, &fields) else { continue };
                Change {
                    entity: "marker",
                    id: id.clone(),
                    name: name_of(&b.fields),
                    class,
                    before: Some(marker_place(a)),
                    after: Some(marker_place(b)),
                    fields,
                }
            }
            (None, None) => continue,
        };
        marks.push(change);
    }
    marks.sort_by(|x, y| {
        let at = |c: &Change| c.after.as_ref().or(c.before.as_ref()).map(|p| p.start).unwrap_or(0.0);
        at(x).total_cmp(&at(y)).then_with(|| x.id.cmp(&y.id))
    });
    out.extend(marks);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Op, apply_batch};
    use serde_json::json;

    fn base() -> Document {
        apply_batch(
            &Document::new("p", "Diff", 1),
            &[
                Op::AddTrack { track: json!({"id": "v", "name": "V", "order": 0, "type": "video"}) },
                Op::AddTrack { track: json!({"id": "w", "name": "W", "order": 1, "type": "video"}) },
                Op::AddItem { item: json!({"id": "a", "trackId": "v", "type": "text", "startTime": 0, "endTime": 4, "name": "Intro"}) },
                Op::AddItem { item: json!({"id": "b", "trackId": "v", "type": "text", "startTime": 4, "endTime": 8}) },
                Op::AddItem { item: json!({"id": "c", "trackId": "v", "type": "text", "startTime": 8, "endTime": 9}) },
                Op::AddMarker { marker: json!({"id": "m", "at": 2}) },
            ],
        )
        .unwrap()
        .doc
    }

    fn after(ops: Vec<Op>) -> Document {
        apply_batch(&base(), &ops).unwrap().doc
    }

    fn set(id: &str, v: Value) -> Op {
        Op::UpdateItem { id: id.into(), set: v.as_object().unwrap().clone() }
    }

    fn one(d: &Document) -> Change {
        let c = semantic_diff(&base(), d);
        assert_eq!(c.len(), 1, "{c:?}");
        c[0].clone()
    }

    #[test]
    fn each_class() {
        let t = one(&after(vec![set("a", json!({"endTime": 3}))]));
        assert_eq!((t.class, t.id.as_str()), (Class::Trimmed, "a"));
        assert_eq!(t.before.unwrap().end, 4.0);
        assert_eq!(t.after.unwrap().end, 3.0);
        assert_eq!(one(&after(vec![set("b", json!({"startTime": 10, "endTime": 14}))])).class, Class::Moved);
        assert_eq!(one(&after(vec![set("b", json!({"trackId": "w"}))])).class, Class::Moved);
        let c = one(&after(vec![set("c", json!({"opacity": 0.5}))]));
        assert_eq!((c.class, c.fields.clone()), (Class::Changed, vec!["opacity".to_string()]));
        assert_eq!(one(&after(vec![Op::RemoveItems { ids: vec!["c".into()] }])).class, Class::Removed);
        let added = after(vec![Op::AddItem {
            item: json!({"id": "d", "trackId": "w", "type": "text", "startTime": 0, "endTime": 1}),
        }]);
        assert_eq!(one(&added).class, Class::Added);
        assert!(semantic_diff(&base(), &base()).is_empty());
    }

    #[test]
    fn markers_are_classified_too() {
        let m = |set: Value| Op::UpdateMarker { id: "m".into(), set: set.as_object().unwrap().clone(), unset: vec![] };
        assert_eq!(one(&after(vec![m(json!({"at": 3}))])).class, Class::Moved);
        assert_eq!(one(&after(vec![m(json!({"dur": 1}))])).class, Class::Trimmed);
        let s = one(&after(vec![m(json!({"status": "review"}))]));
        assert_eq!((s.entity, s.class, s.fields.clone()), ("marker", Class::Changed, vec!["status".to_string()]));
        assert_eq!(one(&after(vec![Op::RemoveMarkers { ids: vec!["m".into()] }])).class, Class::Removed);
    }

    #[test]
    fn lines_read_well() {
        let t = one(&after(vec![set("a", json!({"endTime": 3}))]));
        assert_eq!(t.line(), "trimmed item a \"Intro\": 0–4 s on v → 0–3 s on v");
    }
}
