//! Schema versioning and forward migrations (spec: Schema versioning and migration).
//!
//! Migrations operate on raw JSON so they can restructure anything. Each step upgrades version N to N+1. The store
//! keeps a pre-migration copy (`.revs/pre-migration-<version>.json`) before writing a migrated document.

use serde_json::{Map, Value};

use crate::invariants;
use crate::model::{CURRENT_VERSION, Document, FORMAT_ID, ReadError};
use crate::schema::Severity;

/// How a document was read.
#[derive(Debug, Clone, PartialEq)]
pub enum Opened {
    /// Current version, writable.
    Current(Document),
    /// Older version migrated forward; writable once the pre-migration copy is kept.
    Migrated { doc: Document, from_version: u64 },
    /// Newer than this build supports: open read-only, never write.
    Newer { doc: Option<Document>, version: u64 },
}

impl Opened {
    pub fn doc(&self) -> Option<&Document> {
        match self {
            Opened::Current(d) | Opened::Migrated { doc: d, .. } => Some(d),
            Opened::Newer { doc, .. } => doc.as_ref(),
        }
    }
}

type Step = fn(Value) -> Result<Value, ReadError>;

/// `STEPS[n]` migrates version n to n+1.
const STEPS: &[Step] = &[v0_to_v1];

/// Version 0 (pre-release layout, used as the migration fixture): no `format` field, items under `project.items`,
/// and tracks under `project.trackList`.
fn v0_to_v1(mut v: Value) -> Result<Value, ReadError> {
    let root = v.as_object_mut().ok_or_else(|| ReadError::new("value.type", "$", "must be an object"))?;
    let project = root
        .get_mut("project")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ReadError::new("field.required", "$.project", "is required"))?;
    let mut renamed = Map::new();
    for (k, val) in std::mem::take(project) {
        let key = match k.as_str() {
            "items" => "timelineItems".to_string(),
            "trackList" => "tracks".to_string(),
            _ => k,
        };
        renamed.insert(key, val);
    }
    *project = renamed;
    root.insert("format".into(), FORMAT_ID.into());
    root.insert("version".into(), 1.into());
    Ok(v)
}

/// Parses `project.json` text: migrates older versions, refuses invalid documents with the first invalid field,
/// and returns newer versions read-only.
pub fn open_json(text: &str) -> Result<Opened, ReadError> {
    let v: Value = serde_json::from_str(text).map_err(|e| {
        ReadError::new("json.syntax", format!("line {} column {}", e.line(), e.column()), e.to_string())
    })?;
    open_value(v)
}

pub fn open_value(mut v: Value) -> Result<Opened, ReadError> {
    let version = v
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| ReadError::new("field.required", "$.version", "must be a non-negative integer"))?;
    if version > CURRENT_VERSION {
        // Best effort: a newer document may still parse for read-only viewing.
        let doc = Document::from_json(&v).ok();
        return Ok(Opened::Newer { doc, version });
    }
    let from = version;
    for step in &STEPS[version as usize..] {
        v = step(v)?;
    }
    let doc = Document::from_json(&v)?;
    validate(&doc)?;
    Ok(if from == CURRENT_VERSION { Opened::Current(doc) } else { Opened::Migrated { doc, from_version: from } })
}

/// Fails with the first invariant error, if any.
pub fn validate(doc: &Document) -> Result<(), ReadError> {
    match invariants::check_document(doc).into_iter().find(|i| i.severity == Severity::Error) {
        Some(i) => Err(ReadError { code: i.code, path: i.path, message: i.message }),
        None => Ok(()),
    }
}
