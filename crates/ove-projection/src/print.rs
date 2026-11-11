//! The printer (design D2): deterministic `timeline.tsx` text from a document.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use ove_core::model::{Document, Item, MediaItem};
use ove_core::schema;

use crate::format::{attr_value, micros};
use crate::normal;

/// Projection format marker, written in the header.
pub const FORMAT_LINE: &str = "ove timeline projection, format 1";
/// Maximum line width before attributes wrap one per line.
pub const MAX_WIDTH: usize = 120;

/// Projection tag for an item type.
pub fn tag_for(kind: &str) -> &str {
    if kind == "video" { "clip" } else { kind }
}

/// Item type for a projection tag.
pub fn kind_for(tag: &str) -> Option<&'static str> {
    match tag {
        "clip" => Some("video"),
        t => schema::ITEM_TYPES.iter().find(|k| **k == t && t != "video").copied(),
    }
}

/// Item attribute name for a document field.
pub fn attr_for(field: &str) -> &str {
    match field {
        "startTime" => "start",
        "endTime" => "end",
        "trimStart" => "in",
        "trimEnd" => "out",
        "mediaItemId" => "src",
        f => f,
    }
}

/// Document field for an item attribute name.
pub fn field_for(attr: &str) -> &str {
    match attr {
        "start" => "startTime",
        "end" => "endTime",
        "in" => "trimStart",
        "out" => "trimEnd",
        "src" => "mediaItemId",
        a => a,
    }
}

/// Media names shown in the header and used by `src`: the file stem, de-duplicated with `-2`, `-3`, … in id order.
pub fn media_names(doc: &Document) -> BTreeMap<String, String> {
    let mut taken = BTreeSet::new();
    let mut out = BTreeMap::new();
    for (id, m) in &doc.project.media {
        let base = stem(m).unwrap_or_else(|| id.clone());
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) || (doc.project.media.contains_key(&name) && &name != id) {
            name = format!("{base}-{n}");
            n += 1;
        }
        taken.insert(name.clone());
        out.insert(id.clone(), name);
    }
    out
}

fn stem(m: &MediaItem) -> Option<String> {
    let f = m.file_name()?;
    let s = std::path::Path::new(f).file_stem()?.to_string_lossy().into_owned();
    if s.is_empty() { None } else { Some(s) }
}

/// Extra context the printer shows in the header.
#[derive(Debug, Default, Clone)]
pub struct PrintContext {
    /// Diagnostics lines (warnings from the last apply, verify findings).
    pub diagnostics: Vec<String>,
    /// Media ids whose files are offline.
    pub offline: BTreeSet<String>,
}

fn media_row(m: &MediaItem, doc: &Document, ctx: &PrintContext, name: &str) -> [String; 6] {
    let d = m.data();
    let get = |k: &str| d.and_then(|d| d.get(k));
    let kind = get("type").and_then(Value::as_str).unwrap_or("-").to_string();
    let duration = get("duration")
        .and_then(ove_core::time::json_to_micros)
        .map(|us| format!("{}s", micros(us)))
        .unwrap_or_else(|| "-".into());
    let res = match (get("width").and_then(Value::as_u64), get("height").and_then(Value::as_u64)) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => "-".into(),
    };
    let status = if ctx.offline.contains(&m.id) {
        "offline".to_string()
    } else {
        doc.registry.get(&m.id).map(|e| e.probe_status().to_string()).unwrap_or_else(|| "-".into())
    };
    [name.to_string(), m.id.clone(), kind, duration, res, status]
}

fn header(doc: &Document, ctx: &PrintContext, names: &BTreeMap<String, String>) -> String {
    let mut h = String::new();
    let mut line = |s: &str| {
        if s.is_empty() {
            h.push_str(" *\n");
        } else {
            h.push_str(" * ");
            h.push_str(s);
            h.push('\n');
        }
    };
    line(FORMAT_LINE);
    line("Edit the elements below and apply with `ove source apply`. This header is informational:");
    line("edits to it are ignored, except `rev`, which names the revision this file was printed from.");
    line("");
    line(&format!("aspect: {}", doc.project.aspect_ratio));
    line(&format!("duration: {}s", micros(doc.duration())));
    line(&format!("tracks: {} · items: {}", doc.project.tracks.len(), doc.project.items.len()));
    line("");
    if doc.project.media.is_empty() {
        line("media: (none)");
    } else {
        line("media:");
        let mut rows = vec![["name", "id", "type", "duration", "resolution", "status"].map(String::from)];
        for (id, m) in &doc.project.media {
            rows.push(media_row(m, doc, ctx, &names[id]));
        }
        let widths: Vec<usize> = (0..6).map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap()).collect();
        for r in rows {
            let cells: Vec<String> = r.iter().enumerate().map(|(c, v)| format!("{v:<w$}", w = widths[c])).collect();
            line(&format!("  {}", cells.join("  ").trim_end()));
        }
    }
    let cast: Vec<String> = doc
        .project
        .extra
        .get("cast")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
                    match c.get("role").and_then(Value::as_str) {
                        Some(r) => format!("{name} ({r})"),
                        None => name.to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    line(&format!("cast: {}", if cast.is_empty() { "(none)".into() } else { cast.join(", ") }));
    if ctx.diagnostics.is_empty() {
        line("diagnostics: (none)");
    } else {
        line("diagnostics:");
        for d in &ctx.diagnostics {
            line(&format!("  {}", d.replace("*/", "* /")));
        }
    }
    let pending = doc.project.items.values().filter(|i| i.is_pending()).count();
    line(&format!("pending: {pending}"));
    line(&format!("rev: {}", doc.project.rev));
    format!("/*\n{h} */\n")
}

/// Attributes of an item in print order: id, name, timing, src, content, look, sound, then unknown sorted by key.
pub fn item_attrs(item: &Item, names: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let n = normal::item(item);
    let mut attrs: Vec<(String, String)> = vec![("id".into(), attr_value(&Value::String(n.id.clone())))];
    let mut push = |k: &str, v: &Value| attrs.push((attr_for(k).to_string(), attr_value(v)));
    let known = schema::item_fields(&n.kind);
    let mut emitted = BTreeSet::new();
    for f in known {
        if f.name == "trimStart" {
            // Timing goes after identity (`name`): start, end, then in/out.
            push("startTime", &Value::String(String::new()));
            push("endTime", &Value::String(String::new()));
        }
        if let Some(v) = n.fields.get(f.name) {
            if f.name == "mediaItemId" {
                let id = v.as_str().unwrap_or_default();
                let shown = names.get(id).cloned().unwrap_or_else(|| id.to_string());
                push("mediaItemId", &Value::String(shown));
            } else {
                push(f.name, v);
            }
            emitted.insert(f.name);
        }
    }
    // Fill in the timing placeholders.
    for (k, v) in attrs.iter_mut() {
        if k == "start" {
            *v = format!("{{{}}}", micros(n.start));
        } else if k == "end" {
            *v = format!("{{{}}}", micros(n.end));
        }
    }
    let unknown: Vec<(&String, &Value)> = n.fields.iter().filter(|(k, _)| !emitted.contains(k.as_str())).collect();
    push_unknown(&mut attrs, unknown, ITEM_RESERVED);
    attrs
}

/// Attribute holding unknown fields whose names cannot be attributes.
pub const EXTRA_ATTR: &str = "extraFields";
/// Attribute names with a meaning in projections; unknown fields with these names go to `extraFields`.
pub const ITEM_RESERVED: &[&str] = &[
    "id",
    "start",
    "end",
    "in",
    "out",
    "src",
    "trackId",
    "type",
    "startTime",
    "endTime",
    "trimStart",
    "trimEnd",
    "mediaItemId",
    EXTRA_ATTR,
];
pub const TRACK_RESERVED: &[&str] = &["id", "name", "kind", "order", "type", EXTRA_ATTR];

/// True when a field name can be written as a JSX attribute.
pub fn is_attr_name(k: &str) -> bool {
    let mut c = k.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_' || ch == '$')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || ch == '-')
}

/// Unknown fields sorted by key; names that cannot be attributes are grouped in `extraFields={{…}}`.
fn push_unknown(attrs: &mut Vec<(String, String)>, mut unknown: Vec<(&String, &Value)>, reserved: &[&str]) {
    unknown.sort_by(|a, b| a.0.cmp(b.0));
    let mut extra = serde_json::Map::new();
    for (k, v) in unknown {
        if is_attr_name(k) && !reserved.contains(&k.as_str()) {
            attrs.push((k.clone(), attr_value(v)));
        } else {
            extra.insert(k.clone(), v.clone());
        }
    }
    if !extra.is_empty() {
        attrs.push((EXTRA_ATTR.to_string(), attr_value(&Value::Object(extra))));
    }
}

fn element(out: &mut String, indent: &str, tag: &str, attrs: &[(String, String)], self_close: bool) {
    let close = if self_close { " />" } else { ">" };
    let one = format!("{indent}<{tag}{}{close}", attrs.iter().map(|(k, v)| format!(" {k}={v}")).collect::<String>());
    if one.chars().count() <= MAX_WIDTH || attrs.is_empty() {
        out.push_str(&one);
        out.push('\n');
    } else {
        out.push_str(&format!("{indent}<{tag}\n"));
        for (k, v) in attrs {
            out.push_str(&format!("{indent}  {k}={v}\n"));
        }
        out.push_str(&format!("{indent}{}\n", close.trim_start()));
    }
}

/// Prints the projection of a document.
pub fn print(doc: &Document, ctx: &PrintContext) -> String {
    let names = media_names(doc);
    let mut out = header(doc, ctx, &names);
    out.push_str("export default (\n");
    element(
        &mut out,
        "  ",
        "timeline",
        &[("aspect".into(), attr_value(&Value::String(doc.project.aspect_ratio.clone())))],
        false,
    );
    for t in &doc.project.tracks {
        let t = normal::track(t);
        let mut attrs = vec![
            ("id".to_string(), attr_value(&Value::String(t.id.clone()))),
            ("name".to_string(), attr_value(&Value::String(t.name.clone()))),
            ("kind".to_string(), attr_value(&Value::String(t.kind.clone()))),
        ];
        for f in schema::TRACK_FIELDS {
            if let Some(v) = t.fields.get(f.name) {
                attrs.push((f.name.to_string(), attr_value(v)));
            }
        }
        let unknown: Vec<_> = t.fields.iter().filter(|(k, _)| schema::track_field(k).is_none()).collect();
        push_unknown(&mut attrs, unknown, TRACK_RESERVED);
        element(&mut out, "    ", "track", &attrs, false);
        for item in doc.items_on_track(&t.id) {
            element(&mut out, "      ", tag_for(&item.kind), &item_attrs(item, &names), true);
        }
        out.push_str("    </track>\n");
    }
    out.push_str("  </timeline>\n);\n");
    out
}

/// A line range of a printed projection (`source print --from --lines`). Lines are 1-based.
pub fn slice_lines(text: &str, from: usize, count: Option<usize>) -> String {
    text.lines().skip(from.saturating_sub(1)).take(count.unwrap_or(usize::MAX)).map(|l| format!("{l}\n")).collect()
}
