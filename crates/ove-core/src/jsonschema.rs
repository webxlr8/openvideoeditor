//! JSON Schema and schema reference document generated from the field registry (task 1.3; one source of truth).

use serde_json::{Map, Value, json};

use crate::model::{CURRENT_VERSION, FORMAT_ID};
use crate::schema::{self, Def, Field, Kind};

fn def_json(d: Def) -> Value {
    match d {
        Def::Num(x) if x.fract() == 0.0 => json!(x as i64),
        Def::Num(x) => json!(x),
        Def::Bool(b) => json!(b),
        Def::Str(s) => json!(s),
    }
}

fn kind_schema(k: &Kind) -> Value {
    match k {
        Kind::Str => json!({"type": "string"}),
        Kind::Bool => json!({"type": "boolean"}),
        Kind::Any => json!({}),
        Kind::Num { min, max } => {
            let mut m = Map::new();
            m.insert("type".into(), "number".into());
            if let Some(x) = min {
                m.insert("minimum".into(), json!(x));
            }
            if let Some(x) = max {
                m.insert("maximum".into(), json!(x));
            }
            Value::Object(m)
        }
        Kind::Int { min, max } => {
            let mut m = Map::new();
            m.insert("type".into(), "integer".into());
            if let Some(x) = min {
                m.insert("minimum".into(), json!(x));
            }
            if let Some(x) = max {
                m.insert("maximum".into(), json!(x));
            }
            Value::Object(m)
        }
        Kind::Time => json!({"type": "number", "minimum": 0, "description": "seconds, microsecond precision"}),
        Kind::SignedTime => json!({"type": "number", "description": "seconds, microsecond precision"}),
        Kind::Enum(v) => json!({"enum": v}),
        Kind::Arr(e) => json!({"type": "array", "items": kind_schema(e)}),
        Kind::Obj { fields, open: _ } => {
            // Unknown keys are preserved (closed objects only warn), so additionalProperties stays allowed.
            json!({"type": "object", "properties": props(fields.iter())})
        }
    }
}

fn field_schema(f: &Field) -> Value {
    let mut s = kind_schema(&f.kind);
    if let Value::Object(m) = &mut s {
        if !f.doc.is_empty() {
            m.insert("description".into(), f.doc.into());
        }
        if let Some(d) = f.default {
            m.insert("default".into(), def_json(d));
        }
    }
    s
}

fn props<'a>(fields: impl Iterator<Item = &'a Field>) -> Value {
    Value::Object(fields.map(|f| (f.name.to_string(), field_schema(f))).collect())
}

/// The JSON Schema of `project.json`.
pub fn project_schema() -> Value {
    let item_variants: Vec<Value> = schema::ITEM_TYPES
        .iter()
        .map(|t| {
            let mut p = props(schema::ITEM_HEAD.iter());
            let fields = schema::item_fields(t);
            if let Value::Object(m) = &mut p {
                m.insert("type".into(), json!({"const": t}));
                for f in fields {
                    m.insert(f.name.into(), field_schema(f));
                }
            }
            json!({
                "title": format!("{t} item"),
                "type": "object",
                "required": ["id", "trackId", "type", "startTime", "endTime"],
                "properties": p,
            })
        })
        .collect();
    let mut track_props = props(schema::TRACK_HEAD.iter().chain(schema::TRACK_FIELDS.iter()));
    if let Value::Object(_) = &mut track_props {}
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("{FORMAT_ID}.schema.json"),
        "title": "Project document",
        "description": "Generated from the ove-core field registry. Do not edit by hand (cargo run -p ove-core --bin ove-schema).",
        "type": "object",
        "required": ["format", "version", "project", "media"],
        "properties": {
            "format": {"const": FORMAT_ID},
            "version": {"const": CURRENT_VERSION},
            "project": {
                "type": "object",
                "required": ["id", "name", "stateUpdatedAt", "aspectRatio", "media", "timelineItems", "tracks"],
                "properties": {
                    "id": {"type": "string"},
                    "name": {"type": "string"},
                    "createdAt": {"type": "integer"},
                    "updatedAt": {"type": "integer"},
                    "stateUpdatedAt": {"type": "integer", "minimum": 0},
                    "aspectRatio": {"enum": schema::ASPECT_RATIOS},
                    "media": {"type": "array", "items": {"$ref": "#/$defs/mediaItem"}},
                    "tracks": {"type": "array", "items": {"$ref": "#/$defs/track"}},
                    "timelineItems": {"type": "array", "items": {"$ref": "#/$defs/item"}},
                },
            },
            "media": {"type": "array", "items": {"$ref": "#/$defs/registryEntry"}},
        },
        "$defs": {
            "track": {"type": "object", "required": ["id", "order", "type"], "properties": track_props},
            "mediaItem": {"type": "object", "required": ["id"], "properties": props(schema::MEDIA_ITEM_FIELDS.iter())},
            "registryEntry": {
                "type": "object",
                "required": ["mediaId", "nativePath", "ownedByBundle"],
                "properties": props(schema::REGISTRY_FIELDS.iter()),
            },
            "item": {"oneOf": item_variants},
        }
    })
}

fn kind_text(k: &Kind) -> String {
    match k {
        Kind::Str => "string".into(),
        Kind::Bool => "boolean".into(),
        Kind::Any => "any".into(),
        Kind::Num { min, max } => match (min, max) {
            (None, None) => "number".into(),
            (a, b) => format!(
                "number [{}..{}]",
                a.map(|x| x.to_string()).unwrap_or_default(),
                b.map(|x| x.to_string()).unwrap_or_default()
            ),
        },
        Kind::Int { min, .. } => match min {
            Some(m) => format!("integer ≥ {m}"),
            None => "integer".into(),
        },
        Kind::Time => "seconds ≥ 0".into(),
        Kind::SignedTime => "seconds".into(),
        Kind::Enum(v) => v.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(" \\| "),
        Kind::Arr(e) => format!("array of {}", kind_text(e)),
        Kind::Obj { fields, open } => {
            let keys = fields.iter().map(|f| f.name).collect::<Vec<_>>().join(", ");
            if *open { format!("object {{{keys}, …}}") } else { format!("object {{{keys}}}") }
        }
    }
}

fn table(out: &mut String, fields: &[&Field], required: &[&str]) {
    out.push_str("| field | type / range | default | required | meaning |\n|---|---|---|---|---|\n");
    for f in fields {
        let d = f.default.map(|d| format!("`{}`", def_json(d))).unwrap_or_default();
        let r = if required.contains(&f.name) { "yes" } else { "" };
        out.push_str(&format!("| `{}` | {} | {} | {} | {} |\n", f.name, kind_text(&f.kind), d, r, f.doc));
    }
    out.push('\n');
}

fn nested(out: &mut String, seen: &mut Vec<&'static str>, fields: &[&'static Field]) {
    for f in fields {
        let inner = match f.kind {
            Kind::Obj { fields, .. } => Some(fields),
            Kind::Arr(Kind::Obj { fields, .. }) => Some(*fields),
            _ => None,
        };
        if let Some(inner) = inner {
            if seen.contains(&f.name) {
                continue;
            }
            seen.push(f.name);
            out.push_str(&format!("#### `{}`\n\n", f.name));
            let v: Vec<&Field> = inner.iter().collect();
            table(out, &v, &[]);
            let v2: Vec<&'static Field> = inner.iter().collect();
            nested(out, seen, &v2);
        }
    }
}

/// The schema reference document (`docs/spec/project-schema-v1.md`).
pub fn schema_markdown() -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# Project schema v{CURRENT_VERSION}\n\n\
Generated from the `ove-core` field registry (`cargo run -p ove-core --bin ove-schema`); CI fails if it drifts.\n\
Behaviour contract: docs/spec/document-model.md. Machine-readable form: `docs/spec/project.schema.json`.\n\n\
Conventions:\n\
- Times are seconds as JSON numbers, stored at microsecond precision (integers when whole).\n\
- Fields equal to their default may be omitted; projections always omit them.\n\
- Unknown fields are preserved on read and write, in their original order. Unknown keys inside known closed objects\n  are kept and reported as warnings; open objects (`{{…}}`) take preset parameters.\n\
- Invariant codes are listed in `crates/ove-core/src/invariants.rs`.\n\n\
## Document\n\n\
| field | type | meaning |\n|---|---|---|\n\
| `format` | `\"{FORMAT_ID}\"` | format id |\n\
| `version` | integer | schema version ({CURRENT_VERSION}) |\n\
| `project` | object | see Project |\n\
| `media` | array of registry entries | where media files live and their last known state |\n\n\
## Project\n\n"
    ));
    let pf: Vec<&Field> = schema::PROJECT_FIELDS.iter().collect();
    table(&mut s, &pf, &["id", "name", "stateUpdatedAt", "aspectRatio"]);
    s.push_str("Other project fields (`cast`, `temp`, `mediaBaseDirectory`, …) are preserved and changed with `updateProject`.\n\n## Track\n\n");
    let tf: Vec<&Field> = schema::TRACK_HEAD.iter().chain(schema::TRACK_FIELDS.iter()).collect();
    table(&mut s, &tf, &["id", "order", "type"]);
    s.push_str("## Media item (`project.media[]`)\n\n");
    let mf: Vec<&Field> = schema::MEDIA_ITEM_FIELDS.iter().collect();
    table(&mut s, &mf, &["id"]);
    s.push_str("## Registry entry (`media[]`)\n\n");
    let rf: Vec<&Field> = schema::REGISTRY_FIELDS.iter().collect();
    table(&mut s, &rf, &["mediaId", "nativePath", "ownedByBundle"]);
    s.push_str("## Items\n\nEvery item has these head fields:\n\n");
    let hf: Vec<&Field> = schema::ITEM_HEAD.iter().collect();
    table(&mut s, &hf, &["id", "trackId", "type", "startTime", "endTime"]);
    let mut seen = Vec::new();
    for t in schema::ITEM_TYPES {
        s.push_str(&format!("### `{t}`\n\nFields in projection attribute order:\n\n"));
        let fields = schema::item_fields(t);
        table(&mut s, &fields, &[]);
    }
    s.push_str("### Nested objects\n\n");
    let all: Vec<&'static Field> = schema::ITEM_TYPES.iter().flat_map(|t| schema::item_fields(t)).collect();
    nested(&mut s, &mut seen, &all);
    s
}
