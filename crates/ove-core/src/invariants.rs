//! Document invariants (spec: Document invariants). Every invariant has a stable code.
//!
//! | code | meaning |
//! |---|---|
//! | `track.id.unique` | track ids are unique |
//! | `track.order.contiguous` | track orders are 0..n, unique |
//! | `item.id.unique` | item ids are unique (enforced when adding) |
//! | `item.track.exists` | an item's `trackId` names an existing track |
//! | `item.media.exists` | an item's `mediaItemId` names an existing media item |
//! | `item.time.order` | `startTime < endTime` |
//! | `media.id.unique` | media ids are unique (enforced when adding) |
//! | `registry.media.exists` | a registry entry names an existing media item |
//! | `enum.known` | enum fields hold known values (track/item type, aspect ratio, …) |
//! | `value.type` | a known field has the wrong JSON type |
//! | `value.finite` | a number is finite |
//! | `value.nonNegative` | times, trims, durations and other non-negative values are ≥ 0 |
//! | `value.range` | a number is within its documented range |
//! | `project.format` | the document's format id is this app's |

use std::collections::{BTreeSet, HashSet};

use crate::model::{Document, FORMAT_ID, Item, Track};
use crate::schema::{self, Issue, Kind, Severity};

fn err(code: &'static str, path: String, message: String) -> Issue {
    Issue { severity: Severity::Error, code, path, message }
}

/// Path prefix used for an item in issues: `items[<id>]`.
pub fn item_path(id: &str) -> String {
    format!("items[{id}]")
}

/// Path prefix used for a track in issues: `tracks[<id>]`.
pub fn track_path(id: &str) -> String {
    format!("tracks[{id}]")
}

/// Checks one item (fields and references). Pushes errors and warnings.
pub fn check_item(doc: &Document, item: &Item, out: &mut Vec<Issue>) {
    let p = item_path(&item.id);
    if !schema::ITEM_TYPES.contains(&item.kind.as_str()) {
        out.push(err(
            "enum.known",
            format!("{p}.type"),
            format!("{p}.type must be one of {}, got \"{}\"", schema::ITEM_TYPES.join(", "), item.kind),
        ));
    }
    if doc.track(&item.track_id).is_none() {
        out.push(err(
            "item.track.exists",
            format!("{p}.trackId"),
            format!("item {} is on track \"{}\", which does not exist", item.id, item.track_id),
        ));
    }
    if item.start < 0 {
        out.push(err("value.nonNegative", format!("{p}.startTime"), format!("{p}.startTime must not be negative")));
    }
    if item.start >= item.end {
        out.push(err(
            "item.time.order",
            format!("{p}.endTime"),
            format!("item {} must end after it starts (start < end)", item.id),
        ));
    }
    for (k, v) in &item.fields {
        if let Some(field) = schema::item_field(&item.kind, k) {
            schema::validate(&field.kind, v, &format!("{p}.{k}"), out);
        }
    }
    if let Some(mid) = item.fields.get("mediaItemId").and_then(|v| v.as_str())
        && !doc.project.media.contains_key(mid)
    {
        out.push(err(
            "item.media.exists",
            format!("{p}.mediaItemId"),
            format!("item {} references media \"{mid}\", which does not exist", item.id),
        ));
    }
}

/// Checks one track's fields.
pub fn check_track(track: &Track, out: &mut Vec<Issue>) {
    let p = track_path(&track.id);
    if !schema::TRACK_TYPES.contains(&track.kind.as_str()) {
        out.push(err(
            "enum.known",
            format!("{p}.type"),
            format!("{p}.type must be one of {}, got \"{}\"", schema::TRACK_TYPES.join(", "), track.kind),
        ));
    }
    for (k, v) in &track.fields {
        if let Some(field) = schema::track_field(k) {
            schema::validate(&field.kind, v, &format!("{p}.{k}"), out);
        }
    }
}

/// Global checks: format, aspect ratio, track ids and order, registry and media-item fields.
pub fn check_global(doc: &Document, out: &mut Vec<Issue>) {
    if doc.format != FORMAT_ID {
        out.push(err("project.format", "format".into(), format!("format must be \"{FORMAT_ID}\"")));
    }
    if !schema::ASPECT_RATIOS.contains(&doc.project.aspect_ratio.as_str()) {
        out.push(err(
            "enum.known",
            "project.aspectRatio".into(),
            format!(
                "aspectRatio must be one of {}, got \"{}\"",
                schema::ASPECT_RATIOS.join(", "),
                doc.project.aspect_ratio
            ),
        ));
    }
    let mut ids = HashSet::new();
    for (i, t) in doc.project.tracks.iter().enumerate() {
        if !ids.insert(t.id.as_str()) {
            out.push(err(
                "track.id.unique",
                format!("{}.id", track_path(&t.id)),
                format!("duplicate track id {}", t.id),
            ));
        }
        if t.order as usize != i {
            out.push(err(
                "track.order.contiguous",
                format!("{}.order", track_path(&t.id)),
                "track order values must be unique and contiguous from 0".into(),
            ));
            break;
        }
    }
    for (id, e) in &doc.registry {
        if !doc.project.media.contains_key(id) {
            out.push(err(
                "registry.media.exists",
                format!("media[{id}]"),
                format!("registry entry {id} has no media item"),
            ));
        }
        for (k, v) in &e.fields {
            if let Some(f) = schema::REGISTRY_FIELDS.iter().find(|f| f.name == k) {
                schema::validate(&f.kind, v, &format!("media[{id}].{k}"), out);
            }
        }
    }
    for (id, m) in &doc.project.media {
        for key in ["createdAt", "addedAt"] {
            if let Some(v) = m.fields.get(key) {
                schema::validate(&Kind::Int { min: None, max: None }, v, &format!("project.media[{id}].{key}"), out);
            }
        }
    }
}

/// Checks the whole document. Returns errors and warnings.
pub fn check_document(doc: &Document) -> Vec<Issue> {
    let mut out = Vec::new();
    check_global(doc, &mut out);
    for t in &doc.project.tracks {
        check_track(t, &mut out);
    }
    for item in doc.project.items.values() {
        check_item(doc, item, &mut out);
    }
    out
}

/// Errors only.
pub fn errors(issues: Vec<Issue>) -> Vec<Issue> {
    issues.into_iter().filter(|i| i.severity == Severity::Error).collect()
}

/// Incremental check after a batch: global checks, touched tracks and items fully, and cheap reference checks for
/// every item when a track or media item was removed (that can invalidate untouched items).
pub fn check_touched(
    doc: &Document,
    tracks: &BTreeSet<String>,
    items: &BTreeSet<String>,
    references_may_break: bool,
) -> Vec<Issue> {
    let mut out = Vec::new();
    check_global(doc, &mut out);
    for t in &doc.project.tracks {
        if tracks.contains(&t.id) {
            check_track(t, &mut out);
        }
    }
    if !references_may_break {
        for id in items {
            if let Some(item) = doc.project.items.get(id) {
                check_item(doc, item, &mut out);
            }
        }
        return out;
    }
    for (id, item) in &doc.project.items {
        if items.contains(id) {
            check_item(doc, item, &mut out);
        } else {
            if doc.track(&item.track_id).is_none() {
                out.push(err(
                    "item.track.exists",
                    format!("{}.trackId", item_path(id)),
                    format!("item {id} is on track \"{}\", which does not exist", item.track_id),
                ));
            }
            if let Some(mid) = item.media_id()
                && !doc.project.media.contains_key(mid)
            {
                out.push(err(
                    "item.media.exists",
                    format!("{}.mediaItemId", item_path(id)),
                    format!("item {id} references media \"{mid}\", which does not exist"),
                ));
            }
        }
    }
    out
}
