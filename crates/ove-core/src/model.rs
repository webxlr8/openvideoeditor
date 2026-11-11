//! The canonical project document (`project.json`).
//!
//! Head fields that the core reasons about (ids, references, timing, order) are typed. Everything else lives in an
//! ordered field map per entity, described by the field registry in [`crate::schema`], so unknown fields round-trip
//! unchanged and in their original order.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::schema::{self, Kind};
use crate::time::{Micros, json_to_micros, micros_to_json};

/// Format identifier written to `project.json`.
pub const FORMAT_ID: &str = "ove-project";
/// Schema version this build writes.
pub const CURRENT_VERSION: u64 = 1;

pub type Fields = Map<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub format: String,
    pub version: u64,
    pub project: Project,
    /// Top-level `media` array: registry entries keyed by media id.
    pub registry: BTreeMap<String, RegistryEntry>,
    /// Unknown top-level fields.
    pub extra: Fields,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// `stateUpdatedAt`: the current revision.
    pub rev: i64,
    pub aspect_ratio: String,
    pub media: BTreeMap<String, Arc<MediaItem>>,
    /// Persistent map: cloning a document is O(1) and a batch copies only the paths it changes (design D6).
    pub items: imbl::OrdMap<String, Arc<Item>>,
    /// Sorted by `order`; orders are 0..n.
    pub tracks: Vec<Track>,
    /// Other project fields (cast, temp, mediaBaseDirectory, unknown), in original order.
    pub extra: Fields,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub order: u32,
    pub kind: String,
    /// locked, visible, muted and unknown fields.
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub track_id: String,
    pub kind: String,
    pub start: Micros,
    pub end: Micros,
    /// Every other field (known and unknown), in original order; time-valued known fields are normalised.
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaItem {
    pub id: String,
    /// createdAt, addedAt, data and unknown fields.
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry {
    pub media_id: String,
    pub native_path: String,
    pub owned_by_bundle: bool,
    /// probeStatus, size, modifiedAt and unknown fields.
    pub fields: Fields,
}

/// A structural problem reading a document: the JSON path of the first invalid field and why.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct ReadError {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl ReadError {
    pub fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        ReadError { code, path: path.into(), message: message.into() }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------------------------------------------------

impl Document {
    /// A new empty document.
    pub fn new(id: impl Into<String>, name: impl Into<String>, now_ms: i64) -> Self {
        Document {
            format: FORMAT_ID.into(),
            version: CURRENT_VERSION,
            project: Project {
                id: id.into(),
                name: name.into(),
                created_at: now_ms,
                updated_at: now_ms,
                rev: now_ms,
                aspect_ratio: "16:9".into(),
                media: BTreeMap::new(),
                items: imbl::OrdMap::new(),
                tracks: Vec::new(),
                extra: Fields::new(),
            },
            registry: BTreeMap::new(),
            extra: Fields::new(),
        }
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.project.tracks.iter().find(|t| t.id == id)
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.project.items.get(id).map(|a| a.as_ref())
    }

    /// Duration of the timeline: the latest item end.
    pub fn duration(&self) -> Micros {
        self.project.items.values().map(|i| i.end).max().unwrap_or(0)
    }

    /// Content equality, ignoring revision bookkeeping (`stateUpdatedAt`, `updatedAt`).
    pub fn content_eq(&self, other: &Document) -> bool {
        let mut a = self.clone();
        a.project.rev = other.project.rev;
        a.project.updated_at = other.project.updated_at;
        a == *other
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------------------------

fn obj<'a>(v: &'a Value, path: &str) -> Result<&'a Map<String, Value>, ReadError> {
    v.as_object().ok_or_else(|| ReadError::new("value.type", path, "must be an object"))
}

fn req<'a>(o: &'a Map<String, Value>, key: &str, path: &str) -> Result<&'a Value, ReadError> {
    o.get(key).ok_or_else(|| ReadError::new("field.required", format!("{path}.{key}"), "is required"))
}

fn req_str(o: &Map<String, Value>, key: &str, path: &str) -> Result<String, ReadError> {
    req(o, key, path)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ReadError::new("value.type", format!("{path}.{key}"), "must be a string"))
}

fn req_i64(o: &Map<String, Value>, key: &str, path: &str) -> Result<i64, ReadError> {
    let v = req(o, key, path)?;
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.is_finite()).map(|f| f as i64))
        .ok_or_else(|| ReadError::new("value.type", format!("{path}.{key}"), "must be an integer"))
}

fn opt_i64(o: &Map<String, Value>, key: &str, path: &str) -> Result<i64, ReadError> {
    if o.contains_key(key) { req_i64(o, key, path) } else { Ok(0) }
}

fn req_time(o: &Map<String, Value>, key: &str, path: &str) -> Result<Micros, ReadError> {
    json_to_micros(req(o, key, path)?)
        .ok_or_else(|| ReadError::new("value.type", format!("{path}.{key}"), "must be a finite number of seconds"))
}

fn rest(o: &Map<String, Value>, skip: &[&str]) -> Fields {
    o.iter().filter(|(k, _)| !skip.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Normalises time-valued known fields in an item's field map.
pub fn normalise_item_fields(kind: &str, fields: &mut Fields) {
    for (k, v) in fields.iter_mut() {
        if let Some(f) = schema::item_field(kind, k)
            && needs_normalise(&f.kind)
        {
            *v = schema::normalise(&f.kind, v);
        }
    }
}

fn needs_normalise(k: &Kind) -> bool {
    matches!(k, Kind::Time | Kind::SignedTime | Kind::Arr(_) | Kind::Obj { .. })
}

impl Track {
    pub const HEAD: [&'static str; 4] = ["id", "name", "order", "type"];

    pub fn from_json(v: &Value, path: &str) -> Result<Track, ReadError> {
        let o = obj(v, path)?;
        let order = req_i64(o, "order", path)?;
        if order < 0 || order > u32::MAX as i64 {
            return Err(ReadError::new("value.range", format!("{path}.order"), "must be a non-negative integer"));
        }
        Ok(Track {
            id: req_str(o, "id", path)?,
            name: if o.contains_key("name") { req_str(o, "name", path)? } else { String::new() },
            order: order as u32,
            kind: req_str(o, "type", path)?,
            fields: rest(o, &Self::HEAD),
        })
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), self.id.clone().into());
        m.insert("name".into(), self.name.clone().into());
        m.insert("order".into(), self.order.into());
        m.insert("type".into(), self.kind.clone().into());
        m.extend(self.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(m)
    }

    /// Value of a field by wire name (head or map).
    pub fn get(&self, key: &str) -> Option<Value> {
        match key {
            "id" => Some(self.id.clone().into()),
            "name" => Some(self.name.clone().into()),
            "order" => Some(self.order.into()),
            "type" => Some(self.kind.clone().into()),
            _ => self.fields.get(key).cloned(),
        }
    }
}

impl Item {
    pub const HEAD: [&'static str; 5] = ["id", "trackId", "type", "startTime", "endTime"];

    pub fn from_json(v: &Value, path: &str) -> Result<Item, ReadError> {
        let o = obj(v, path)?;
        let kind = req_str(o, "type", path)?;
        let mut fields = rest(o, &Self::HEAD);
        normalise_item_fields(&kind, &mut fields);
        Ok(Item {
            id: req_str(o, "id", path)?,
            track_id: req_str(o, "trackId", path)?,
            kind,
            start: req_time(o, "startTime", path)?,
            end: req_time(o, "endTime", path)?,
            fields,
        })
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), self.id.clone().into());
        m.insert("trackId".into(), self.track_id.clone().into());
        m.insert("type".into(), self.kind.clone().into());
        m.insert("startTime".into(), micros_to_json(self.start));
        m.insert("endTime".into(), micros_to_json(self.end));
        m.extend(self.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(m)
    }

    /// Value of a field by wire name (head or map).
    pub fn get(&self, key: &str) -> Option<Value> {
        match key {
            "id" => Some(self.id.clone().into()),
            "trackId" => Some(self.track_id.clone().into()),
            "type" => Some(self.kind.clone().into()),
            "startTime" => Some(micros_to_json(self.start)),
            "endTime" => Some(micros_to_json(self.end)),
            _ => self.fields.get(key).cloned(),
        }
    }

    pub fn name(&self) -> &str {
        self.fields.get("name").and_then(Value::as_str).unwrap_or("")
    }

    pub fn media_id(&self) -> Option<&str> {
        self.fields.get("mediaItemId").and_then(Value::as_str)
    }

    pub fn is_pending(&self) -> bool {
        self.fields.get("pending").and_then(Value::as_bool).unwrap_or(false)
    }

    /// A time-valued field in microseconds (0 when absent).
    pub fn time_field(&self, key: &str) -> Micros {
        self.fields.get(key).and_then(json_to_micros).unwrap_or(0)
    }
}

impl MediaItem {
    pub fn from_json(v: &Value, path: &str) -> Result<MediaItem, ReadError> {
        let o = obj(v, path)?;
        Ok(MediaItem { id: req_str(o, "id", path)?, fields: rest(o, &["id"]) })
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), self.id.clone().into());
        m.extend(self.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(m)
    }

    pub fn data(&self) -> Option<&Map<String, Value>> {
        self.fields.get("data").and_then(Value::as_object)
    }

    /// A display name: data.fileName without extension, else the id.
    pub fn file_name(&self) -> Option<&str> {
        self.data().and_then(|d| d.get("fileName")).and_then(Value::as_str)
    }
}

impl RegistryEntry {
    pub const HEAD: [&'static str; 3] = ["mediaId", "nativePath", "ownedByBundle"];

    pub fn from_json(v: &Value, path: &str) -> Result<RegistryEntry, ReadError> {
        let o = obj(v, path)?;
        let owned = req(o, "ownedByBundle", path)?
            .as_bool()
            .ok_or_else(|| ReadError::new("value.type", format!("{path}.ownedByBundle"), "must be true or false"))?;
        Ok(RegistryEntry {
            media_id: req_str(o, "mediaId", path)?,
            native_path: req_str(o, "nativePath", path)?,
            owned_by_bundle: owned,
            fields: rest(o, &Self::HEAD),
        })
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("mediaId".into(), self.media_id.clone().into());
        m.insert("nativePath".into(), self.native_path.clone().into());
        m.insert("ownedByBundle".into(), self.owned_by_bundle.into());
        m.extend(self.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(m)
    }

    pub fn probe_status(&self) -> &str {
        self.fields.get("probeStatus").and_then(Value::as_str).unwrap_or("awaiting-probe")
    }
}

impl Document {
    /// Reads the structure of a current-version document. Does not check invariants (see
    /// [`crate::invariants::check_document`]) and does not migrate (see [`crate::migrate`]).
    pub fn from_json(v: &Value) -> Result<Document, ReadError> {
        let root = obj(v, "$")?;
        let format = req_str(root, "format", "$")?;
        let version = req(root, "version", "$")?
            .as_u64()
            .ok_or_else(|| ReadError::new("value.type", "$.version", "must be a non-negative integer"))?;
        let p = obj(req(root, "project", "$")?, "$.project")?;
        let pp = "$.project";

        let mut media = BTreeMap::new();
        if let Some(arr) = p.get("media") {
            let arr =
                arr.as_array().ok_or_else(|| ReadError::new("value.type", "$.project.media", "must be an array"))?;
            for (i, m) in arr.iter().enumerate() {
                let path = format!("$.project.media[{i}]");
                let mi = MediaItem::from_json(m, &path)?;
                if media.insert(mi.id.clone(), Arc::new(mi)).is_some() {
                    return Err(ReadError::new("media.id.unique", format!("{path}.id"), "duplicate media id"));
                }
            }
        }

        let mut tracks = Vec::new();
        if let Some(arr) = p.get("tracks") {
            let arr =
                arr.as_array().ok_or_else(|| ReadError::new("value.type", "$.project.tracks", "must be an array"))?;
            for (i, t) in arr.iter().enumerate() {
                tracks.push(Track::from_json(t, &format!("$.project.tracks[{i}]"))?);
            }
        }
        tracks.sort_by_key(|t| t.order);

        let mut items = imbl::OrdMap::new();
        if let Some(arr) = p.get("timelineItems") {
            let arr = arr
                .as_array()
                .ok_or_else(|| ReadError::new("value.type", "$.project.timelineItems", "must be an array"))?;
            for (i, it) in arr.iter().enumerate() {
                let path = format!("$.project.timelineItems[{i}]");
                let item = Item::from_json(it, &path)?;
                if items.insert(item.id.clone(), Arc::new(item)).is_some() {
                    return Err(ReadError::new("item.id.unique", format!("{path}.id"), "duplicate item id"));
                }
            }
        }

        let mut registry = BTreeMap::new();
        if let Some(arr) = root.get("media") {
            let arr = arr.as_array().ok_or_else(|| ReadError::new("value.type", "$.media", "must be an array"))?;
            for (i, e) in arr.iter().enumerate() {
                let path = format!("$.media[{i}]");
                let entry = RegistryEntry::from_json(e, &path)?;
                if registry.insert(entry.media_id.clone(), entry).is_some() {
                    return Err(ReadError::new(
                        "media.id.unique",
                        format!("{path}.mediaId"),
                        "duplicate registry entry",
                    ));
                }
            }
        }

        Ok(Document {
            format,
            version,
            project: Project {
                id: req_str(p, "id", pp)?,
                name: req_str(p, "name", pp)?,
                created_at: opt_i64(p, "createdAt", pp)?,
                updated_at: opt_i64(p, "updatedAt", pp)?,
                rev: req_i64(p, "stateUpdatedAt", pp)?,
                aspect_ratio: req_str(p, "aspectRatio", pp)?,
                media,
                items,
                tracks,
                extra: rest(
                    p,
                    &[
                        "id",
                        "name",
                        "createdAt",
                        "updatedAt",
                        "stateUpdatedAt",
                        "aspectRatio",
                        "media",
                        "timelineItems",
                        "tracks",
                    ],
                ),
            },
            registry,
            extra: rest(root, &["format", "version", "project", "media"]),
        })
    }

    pub fn to_json(&self) -> Value {
        let p = &self.project;
        let mut pm = Map::new();
        pm.insert("id".into(), p.id.clone().into());
        pm.insert("name".into(), p.name.clone().into());
        pm.insert("createdAt".into(), p.created_at.into());
        pm.insert("updatedAt".into(), p.updated_at.into());
        pm.insert("stateUpdatedAt".into(), p.rev.into());
        pm.insert("aspectRatio".into(), p.aspect_ratio.clone().into());
        pm.extend(p.extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        pm.insert("media".into(), Value::Array(p.media.values().map(|m| m.to_json()).collect()));
        pm.insert("tracks".into(), Value::Array(p.tracks.iter().map(Track::to_json).collect()));
        pm.insert("timelineItems".into(), Value::Array(self.items_in_canonical_order().map(|i| i.to_json()).collect()));

        let mut root = Map::new();
        root.insert("format".into(), self.format.clone().into());
        root.insert("version".into(), self.version.into());
        root.insert("project".into(), Value::Object(pm));
        root.insert("media".into(), Value::Array(self.registry.values().map(RegistryEntry::to_json).collect()));
        root.extend(self.extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(root)
    }

    /// Items ordered by track order, then start time, then id.
    pub fn items_in_canonical_order(&self) -> impl Iterator<Item = &Item> {
        let order: BTreeMap<&str, u32> = self.project.tracks.iter().map(|t| (t.id.as_str(), t.order)).collect();
        let mut v: Vec<&Item> = self.project.items.values().map(|a| a.as_ref()).collect();
        v.sort_by(|a, b| {
            let oa = order.get(a.track_id.as_str()).copied().unwrap_or(u32::MAX);
            let ob = order.get(b.track_id.as_str()).copied().unwrap_or(u32::MAX);
            (oa, a.start, &a.id).cmp(&(ob, b.start, &b.id))
        });
        v.into_iter()
    }

    /// Pretty JSON text as written to disk (2-space indent, trailing newline).
    pub fn to_string_pretty(&self) -> String {
        let mut s = serde_json::to_string_pretty(&self.to_json()).expect("serialisable");
        s.push('\n');
        s
    }

    /// Items on a track, sorted by start then id.
    pub fn items_on_track(&self, track_id: &str) -> Vec<&Item> {
        let mut v: Vec<&Item> =
            self.project.items.values().map(|a| a.as_ref()).filter(|i| i.track_id == track_id).collect();
        v.sort_by(|a, b| (a.start, &a.id).cmp(&(b.start, &b.id)));
        v
    }
}
