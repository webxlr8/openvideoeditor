//! Named versions (add-markers-keyframe-lanes-and-versions D8): a name pinned to a revision with a full snapshot that
//! pruning never removes, so "version 2 sent to the client" can be printed, compared and restored long after its
//! revision records are gone.
//!
//! Layout, under `.revs/versions/`:
//! - `objects/<sha256>.json`: one item each, content-addressed, so items unchanged between versions are stored once
//!   (shared structure);
//! - `<rev>.json`: the version's snapshot: the document without its items, plus the ids of its item objects;
//! - `../versions.json`: the list of versions `{name, rev, createdAt, actor, note, bytes}`, written last.
//!
//! A save writes objects, then the snapshot, then the list, each atomically, so a crash during a save leaves the
//! version either absent or complete. Pruning only touches revision records and periodic snapshots, never this
//! folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ove_core::model::Document;
use ove_core::revision::{Actor, Source};
use ove_core::semdiff::{Change, semantic_diff};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Result, StoreError};
use crate::fsio::{self, Points};
use crate::project::{Committed, Project};

/// Failpoints of the three writes of a save (exercised by the fault-injection test).
pub const OBJECT: Points = Points {
    write: "version.object.write",
    flush: "version.object.flush",
    rename: "version.object.rename",
    dirsync: "version.object.dirsync",
};
pub const SNAPSHOT: Points = Points {
    write: "version.snap.write",
    flush: "version.snap.flush",
    rename: "version.snap.rename",
    dirsync: "version.snap.dirsync",
};
pub const LIST: Points =
    Points { write: "versions.write", flush: "versions.flush", rename: "versions.rename", dirsync: "versions.dirsync" };

/// Longest version name, in characters.
pub const NAME_MAX: usize = 120;

/// One named version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub name: String,
    pub rev: i64,
    /// ms since the epoch.
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Bytes this version added to the bundle (its snapshot plus the item objects no earlier version had).
    #[serde(default)]
    pub bytes: u64,
}

fn dir(bundle: &Path) -> PathBuf {
    bundle.join(".revs").join("versions")
}

fn list_path(bundle: &Path) -> PathBuf {
    bundle.join(".revs").join("versions.json")
}

fn snap_path(bundle: &Path, rev: i64) -> PathBuf {
    dir(bundle).join(format!("{rev}.json"))
}

fn object_path(bundle: &Path, id: &str) -> PathBuf {
    dir(bundle).join("objects").join(format!("{id}.json"))
}

fn io(op: &str, path: &Path, e: std::io::Error) -> StoreError {
    StoreError::io(op, path, e)
}

/// The versions of a bundle, oldest first (empty when none were saved).
pub fn list(bundle: &Path) -> Result<Vec<Version>> {
    let path = list_path(bundle);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| StoreError::refused("version.invalid", format!("{} is not readable: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(io("read", &path, e)),
    }
}

/// Finds a version by name.
pub fn find(bundle: &Path, name: &str) -> Result<Version> {
    list(bundle)?
        .into_iter()
        .find(|v| v.name == name)
        .ok_or_else(|| StoreError::refused("version.missing", format!("there is no version named \"{name}\"")))
}

/// Reads a version's document.
pub fn doc(bundle: &Path, v: &Version) -> Result<Document> {
    let path = snap_path(bundle, v.rev);
    let bytes = std::fs::read(&path).map_err(|e| io("read", &path, e))?;
    let snap: Value = serde_json::from_slice(&bytes)
        .map_err(|e| StoreError::refused("version.invalid", format!("{} is not readable: {e}", path.display())))?;
    let mut d = snap.get("doc").cloned().unwrap_or(Value::Null);
    let mut items = Vec::new();
    for id in snap.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let p = object_path(bundle, id);
        let b = std::fs::read(&p).map_err(|e| io("read", &p, e))?;
        items
            .push(serde_json::from_slice::<Value>(&b).map_err(|e| {
                StoreError::refused("version.invalid", format!("{} is not readable: {e}", p.display()))
            })?);
    }
    if let Some(p) = d.get_mut("project").and_then(Value::as_object_mut) {
        p.insert("timelineItems".into(), Value::Array(items));
    }
    Document::from_json(&d).map_err(|e| StoreError::refused("version.invalid", e.to_string()))
}

fn object_id(text: &str) -> String {
    let h = Sha256::digest(text.as_bytes());
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes the snapshot of `d` (objects first, then the snapshot). Returns the bytes written.
fn write_snapshot(bundle: &Path, d: &Document) -> Result<u64> {
    let mut written = 0u64;
    let mut full = d.to_json();
    let items: Vec<Value> = full
        .get_mut("project")
        .and_then(Value::as_object_mut)
        .and_then(|p| p.remove("timelineItems"))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    std::fs::create_dir_all(dir(bundle).join("objects")).map_err(|e| io("create", &dir(bundle), e))?;
    let mut ids = Vec::with_capacity(items.len());
    let mut seen = BTreeSet::new();
    for it in &items {
        let text = format!("{it}");
        let id = object_id(&text);
        let p = object_path(bundle, &id);
        if seen.insert(id.clone()) && !p.exists() {
            fsio::write_atomic(&p, text.as_bytes(), OBJECT, false).map_err(|e| io("write", &p, e))?;
            written += text.len() as u64;
        }
        ids.push(Value::String(id));
    }
    let text = format!("{:#}\n", json!({"format": "ove-version-snapshot", "doc": full, "items": ids}));
    let p = snap_path(bundle, d.project.rev);
    if !p.exists() {
        fsio::write_atomic(&p, text.as_bytes(), SNAPSHOT, false).map_err(|e| io("write", &p, e))?;
        written += text.len() as u64;
    }
    Ok(written)
}

fn write_list(bundle: &Path, list: &[Version]) -> Result<()> {
    let p = list_path(bundle);
    let text = serde_json::to_vec_pretty(list).map_err(std::io::Error::from).map_err(|e| io("write", &p, e))?;
    fsio::write_atomic(&p, &text, LIST, false).map_err(|e| io("write", &p, e))
}

impl Project {
    /// The project's named versions, oldest first.
    pub fn versions(&self) -> Result<Vec<Version>> {
        list(self.bundle())
    }

    /// Saves the current revision as a named version with a snapshot that pruning never removes. A name already in
    /// use is refused with `version.exists`.
    pub fn save_version(&self, name: &str, note: Option<&str>, actor: Option<Actor>, now_ms: i64) -> Result<Version> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > NAME_MAX {
            return Err(StoreError::refused(
                "version.name",
                format!("a version name needs 1 to {NAME_MAX} characters"),
            ));
        }
        let mut all = self.versions()?;
        if all.iter().any(|v| v.name == name) {
            return Err(StoreError::refused("version.exists", format!("a version named \"{name}\" already exists")));
        }
        let bytes = write_snapshot(self.bundle(), self.doc())?;
        let v = Version {
            name: name.to_string(),
            rev: self.rev(),
            created_at: now_ms,
            actor,
            note: note.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string),
            bytes,
        };
        all.push(v.clone());
        write_list(self.bundle(), &all)?;
        Ok(v)
    }

    /// The document of a named version.
    pub fn version_doc(&self, name: &str) -> Result<Document> {
        doc(self.bundle(), &find(self.bundle(), name)?)
    }

    /// Classified changes from version `from` to version `to`, or to the current document when `to` is `None`.
    pub fn version_diff(&self, from: &str, to: Option<&str>) -> Result<Vec<Change>> {
        let a = self.version_doc(from)?;
        let b = match to {
            Some(n) => self.version_doc(n)?,
            None => self.doc().clone(),
        };
        Ok(semantic_diff(&a, &b))
    }

    /// Restores a named version as one new revision (undoable like any other). `Ok(None)` when the document already
    /// equals the version.
    pub fn restore_version(&mut self, name: &str, actor: Option<Actor>) -> Result<Option<Committed>> {
        let target = self.version_doc(name)?;
        let ops = ove_core::diff::diff_documents(self.doc(), &target);
        if ops.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.apply_as(ops, Source::Restore, Some(format!("restore version \"{name}\"")), actor)?))
    }
}
