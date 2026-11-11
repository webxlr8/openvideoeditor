//! Normalisation: the canonical form compared by round trip and merge. Fields equal to their defaults are removed,
//! default keys inside known objects are removed (and known objects left empty are dropped), and integral floats
//! become integers so `100.0` and `100` compare equal.

use serde_json::{Map, Number, Value};

use ove_core::model::{Document, Fields, Item, Track};
use ove_core::schema;

/// Canonical numbers: integral floats become integers, recursively.
pub fn canon(v: &Value) -> Value {
    match v {
        Value::Number(n) if n.as_i64().is_none() && n.as_u64().is_none() => {
            let f = n.as_f64().unwrap_or(0.0);
            if f == f.trunc() && f.abs() < 9.0e15 { Value::Number(Number::from(f as i64)) } else { v.clone() }
        }
        Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), canon(v))).collect()),
        _ => v.clone(),
    }
}

/// Normalised item field map.
pub fn item_fields(kind: &str, fields: &Fields) -> Fields {
    let mut out = Map::new();
    for (k, v) in fields {
        let v = canon(v);
        match schema::item_field(kind, k) {
            Some(f) if schema::is_default(f, &v) => {}
            Some(f) => {
                let s = schema::strip_defaults(&f.kind, &v);
                if matches!(f.kind, schema::Kind::Obj { .. }) && s.as_object().is_some_and(|m| m.is_empty()) {
                    continue;
                }
                out.insert(k.clone(), s);
            }
            None => {
                out.insert(k.clone(), v);
            }
        }
    }
    out
}

pub fn item(i: &Item) -> Item {
    Item { fields: item_fields(&i.kind, &i.fields), ..i.clone() }
}

pub fn track_fields(fields: &Fields) -> Fields {
    fields
        .iter()
        .map(|(k, v)| (k.clone(), canon(v)))
        .filter(|(k, v)| !schema::track_field(k).is_some_and(|f| schema::is_default(f, v)))
        .collect()
}

pub fn track(t: &Track) -> Track {
    Track { fields: track_fields(&t.fields), ..t.clone() }
}

/// Normalised document (only tracks and items change; media and project fields are kept as they are).
pub fn document(d: &Document) -> Document {
    let mut out = d.clone();
    out.project.tracks = d.project.tracks.iter().map(track).collect();
    out.project.items = d.project.items.iter().map(|(id, i)| (id.clone(), std::sync::Arc::new(item(i)))).collect();
    out
}
