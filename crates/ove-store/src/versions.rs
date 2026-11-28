//! Named versions (add-markers-keyframe-lanes-and-versions D8): a name pinned to a revision with a full snapshot that
//! pruning never removes, so "version 2 sent to the client" can be printed, compared and restored long after its
//! revision records are gone.
//!
//! Layout, under `.revs/versions/`, where `<id>` is `<rev>-<n>` (n: how many versions existed before it):
//! - `packs/<id>.items.json` and `packs/<id>.hashes.json`: the items this version stored that no earlier version
//!   had, and their content hashes. Items unchanged between versions are stored once (shared structure).
//! - `<id>.json`: the snapshot: the document without its items, plus the items as ranges `[pack, start, count]` of
//!   pack entries, in document order. Unchanged runs of items collapse into a few ranges.
//! - `../versions.json`: the list of versions `{name, rev, snapshot, createdAt, actor, note, bytes}`, written last.
//!
//! A save is four atomic writes (pack items, pack hashes, snapshot, list), so a crash during a save leaves the version
//! either absent or complete; files a failed save left behind are overwritten by the next save with that id. Pruning
//! only touches revision records and periodic snapshots, never this folder.

use std::collections::{BTreeMap, HashMap};
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

/// Failpoints of the writes of a save (exercised by the fault-injection test).
pub const PACK: Points = Points {
    write: "version.pack.write",
    flush: "version.pack.flush",
    rename: "version.pack.rename",
    dirsync: "version.pack.dirsync",
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
    /// The snapshot id (`<rev>-<n>`).
    #[serde(default)]
    pub snapshot: String,
    /// ms since the epoch.
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Bytes this version added to the bundle (its snapshot plus the items no earlier version had).
    #[serde(default)]
    pub bytes: u64,
}

fn dir(bundle: &Path) -> PathBuf {
    bundle.join(".revs").join("versions")
}

fn list_path(bundle: &Path) -> PathBuf {
    bundle.join(".revs").join("versions.json")
}

fn snap_path(bundle: &Path, id: &str) -> PathBuf {
    dir(bundle).join(format!("{id}.json"))
}

fn pack_path(bundle: &Path, id: &str, part: &str) -> PathBuf {
    dir(bundle).join("packs").join(format!("{id}.{part}.json"))
}

fn io(op: &str, path: &Path, e: std::io::Error) -> StoreError {
    StoreError::io(op, path, e)
}

fn invalid(path: &Path, e: impl std::fmt::Display) -> StoreError {
    StoreError::refused("version.invalid", format!("{} is not readable: {e}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|e| io("read", path, e))?;
    serde_json::from_slice(&bytes).map_err(|e| invalid(path, e))
}

/// The versions of a bundle, oldest first (empty when none were saved).
pub fn list(bundle: &Path) -> Result<Vec<Version>> {
    let path = list_path(bundle);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| invalid(&path, e)),
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
    let path = snap_path(bundle, &v.snapshot);
    let snap: Value = read_json(&path)?;
    let mut d = snap.get("doc").cloned().unwrap_or(Value::Null);
    let mut packs: HashMap<String, Vec<Value>> = HashMap::new();
    let mut items = Vec::new();
    for r in snap.get("items").and_then(Value::as_array).into_iter().flatten() {
        let (Some(pack), Some(start), Some(count)) =
            (r.get(0).and_then(Value::as_str), r.get(1).and_then(Value::as_u64), r.get(2).and_then(Value::as_u64))
        else {
            return Err(invalid(&path, "an item range is malformed"));
        };
        if !packs.contains_key(pack) {
            let list: Vec<Value> = read_json(&pack_path(bundle, pack, "items"))?;
            packs.insert(pack.to_string(), list);
        }
        let list = &packs[pack];
        let (a, b) = (start as usize, (start + count) as usize);
        let run = list.get(a..b).ok_or_else(|| invalid(&path, format!("pack {pack} has no entries {a}..{b}")))?;
        items.extend(run.iter().cloned());
    }
    if let Some(p) = d.get_mut("project").and_then(Value::as_object_mut) {
        p.insert("timelineItems".into(), Value::Array(items));
    }
    Document::from_json(&d).map_err(|e| StoreError::refused("version.invalid", e.to_string()))
}

/// A short content hash: the first 16 bytes of SHA-256, as hex.
fn content_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Where every item stored by the listed versions lives: hash → (pack, index).
fn stored(bundle: &Path, versions: &[Version]) -> Result<HashMap<String, (String, u64)>> {
    let mut m = HashMap::new();
    for v in versions {
        let p = pack_path(bundle, &v.snapshot, "hashes");
        let hashes: Vec<String> = read_json(&p)?;
        for (i, h) in hashes.into_iter().enumerate() {
            m.entry(h).or_insert_with(|| (v.snapshot.clone(), i as u64));
        }
    }
    Ok(m)
}

/// Writes the pack and snapshot of `d` under `id`, sharing items the earlier versions stored. Returns the bytes
/// written.
fn write_snapshot(bundle: &Path, id: &str, d: &Document, earlier: &[Version]) -> Result<u64> {
    let mut known = stored(bundle, earlier)?;
    let mut full = d.to_json();
    let items: Vec<Value> = full
        .get_mut("project")
        .and_then(Value::as_object_mut)
        .and_then(|p| p.remove("timelineItems"))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let (mut new_items, mut new_hashes) = (Vec::new(), Vec::new());
    let mut ranges: Vec<(String, u64, u64)> = Vec::new();
    for it in items {
        let h = content_hash(&it.to_string());
        let (pack, at) = match known.get(&h) {
            Some(loc) => loc.clone(),
            None => {
                let loc = (id.to_string(), new_items.len() as u64);
                known.insert(h.clone(), loc.clone());
                new_hashes.push(h);
                new_items.push(it);
                loc
            }
        };
        match ranges.last_mut() {
            Some((p, s, n)) if *p == pack && *s + *n == at => *n += 1,
            _ => ranges.push((pack, at, 1)),
        }
    }
    std::fs::create_dir_all(dir(bundle).join("packs")).map_err(|e| io("create", &dir(bundle), e))?;
    let mut written = 0u64;
    for (part, value) in [("items", Value::Array(new_items)), ("hashes", json!(new_hashes))] {
        let p = pack_path(bundle, id, part);
        let text = value.to_string();
        fsio::write_atomic(&p, text.as_bytes(), PACK, false).map_err(|e| io("write", &p, e))?;
        written += text.len() as u64;
    }
    let ranges: Vec<Value> = ranges.into_iter().map(|(p, s, n)| json!([p, s, n])).collect();
    let text = json!({"format": "ove-version-snapshot", "doc": full, "items": ranges}).to_string();
    let p = snap_path(bundle, id);
    fsio::write_atomic(&p, text.as_bytes(), SNAPSHOT, false).map_err(|e| io("write", &p, e))?;
    Ok(written + text.len() as u64)
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
        let id = format!("{}-{}", self.rev(), all.len());
        let bytes = write_snapshot(self.bundle(), &id, self.doc(), &all)?;
        let v = Version {
            name: name.to_string(),
            rev: self.rev(),
            snapshot: id,
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

/// Pack entries each listed version stored, by snapshot id (for size reports).
pub fn pack_sizes(bundle: &Path) -> Result<BTreeMap<String, usize>> {
    let mut m = BTreeMap::new();
    for v in list(bundle)? {
        let hashes: Vec<String> = read_json(&pack_path(bundle, &v.snapshot, "hashes"))?;
        m.insert(v.snapshot, hashes.len());
    }
    Ok(m)
}
