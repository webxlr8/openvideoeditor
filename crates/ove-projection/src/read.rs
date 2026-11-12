//! Reads a parsed projection into a file model: tag/attribute mapping, value validation against the field
//! registry, and `src` resolution (design D7).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use ove_core::model::{Document, Fields};
use ove_core::schema::{self, Severity as SchemaSeverity};
use ove_core::time::{Micros, json_to_micros};

use crate::normal;
use crate::parse::{Attr, Diagnostic, Element, Pos, Severity};
use crate::print::{field_for, kind_for, media_names};

#[derive(Debug, Clone, PartialEq)]
pub struct FileTrack {
    /// `None` for a new track without an id.
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub fields: Fields,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileItem {
    pub id: Option<String>,
    pub kind: String,
    /// Index into `FileModel::tracks`.
    pub track: usize,
    pub start: Micros,
    pub end: Micros,
    /// Normalised fields with document names (`trimStart`, `mediaItemId`, …).
    pub fields: Fields,
    pub pos: Pos,
}

/// A new media file referenced by path.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMedia {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileModel {
    pub aspect: Option<String>,
    pub tracks: Vec<FileTrack>,
    pub items: Vec<FileItem>,
    pub new_media: Vec<NewMedia>,
}

/// What `src` and paths resolve against.
#[derive(Debug, Clone, Copy)]
pub struct Resolve<'a> {
    pub base: &'a Document,
    pub current: &'a Document,
    /// Bundle folder for relative paths and owned media. `None` resolves relative paths against the working dir.
    pub bundle: Option<&'a Path>,
}

struct Reader<'a> {
    r: Resolve<'a>,
    diags: Vec<Diagnostic>,
    base_names: BTreeMap<String, String>,
    cur_names: BTreeMap<String, String>,
    known_paths: BTreeMap<PathBuf, String>,
    new_media: Vec<NewMedia>,
}

fn diag(sev: Severity, pos: Pos, code: &str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic { severity: sev, line: pos.line, column: pos.column, code: code.into(), message: msg.into() }
}

/// Expands `~/` and resolves relative paths against the bundle.
pub fn resolve_path(bundle: Option<&Path>, s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\"))
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    let p = PathBuf::from(s);
    if p.is_absolute() {
        p
    } else {
        match bundle {
            Some(b) => b.join(p),
            None => std::env::current_dir().map(|d| d.join(&p)).unwrap_or(p),
        }
    }
}

/// Canonical form for comparing paths: resolved symlinks where the file exists; case-folded on Windows and macOS
/// (case-insensitive by default).
pub fn path_key(p: &Path) -> PathBuf {
    let c = dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if cfg!(any(windows, target_os = "macos")) { PathBuf::from(c.to_string_lossy().to_lowercase()) } else { c }
}

fn media_type_for(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "tif" | "tiff" | "heic" | "bmp" | "svg" => "image",
        "wav" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "aif" | "aiff" | "caf" => "audio",
        _ => "video",
    }
}

/// Stable id for a new media path (FNV-1a), so applying the same file twice registers it once.
fn media_id_for(path: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("m_{:012x}", h & 0xffff_ffff_ffff)
}

impl Reader<'_> {
    fn push(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    fn resolve_src(&mut self, a: &Attr) -> Option<String> {
        let Some(s) = a.value.as_str() else {
            self.push(diag(
                Severity::Error,
                a.value_pos,
                "value.type",
                "src must be a string: a media name, media id or file path",
            ));
            return None;
        };
        for names in [&self.base_names, &self.cur_names] {
            if let Some((id, _)) = names.iter().find(|(_, n)| n.as_str() == s) {
                return Some(id.clone());
            }
        }
        if self.r.current.project.media.contains_key(s) || self.r.base.project.media.contains_key(s) {
            return Some(s.to_string());
        }
        let path = resolve_path(self.r.bundle, s);
        if !path.is_file() {
            self.push(diag(
                Severity::Error,
                a.value_pos,
                "src.unresolved",
                format!("src \"{s}\" is not a media name or id, and no file exists at {}", path.display()),
            ));
            return None;
        }
        let key = path_key(&path);
        if let Some(id) = self.known_paths.get(&key) {
            return Some(id.clone());
        }
        let id = media_id_for(&key);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| id.clone());
        let taken: BTreeSet<&String> = self.cur_names.values().chain(self.new_media.iter().map(|m| &m.name)).collect();
        let mut name = stem.clone();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{stem}-{n}");
            n += 1;
        }
        self.known_paths.insert(key, id.clone());
        self.new_media.push(NewMedia { id: id.clone(), path: dunce::canonicalize(&path).unwrap_or(path), name });
        Some(id)
    }

    /// Validates a known field's value and reports errors/warnings at the attribute.
    fn check_value(&mut self, field: &schema::Field, attr: &Attr) -> bool {
        let mut issues = Vec::new();
        schema::validate(&field.kind, &attr.value, &attr.name, &mut issues);
        let mut ok = true;
        for i in issues {
            let sev = if i.severity == SchemaSeverity::Error {
                ok = false;
                Severity::Error
            } else {
                Severity::Warning
            };
            self.push(diag(sev, attr.value_pos, i.code, i.message));
        }
        ok
    }

    fn track(&mut self, el: &Element) -> Option<FileTrack> {
        let mut t = FileTrack { id: None, name: String::new(), kind: String::new(), fields: Map::new(), pos: el.pos };
        let mut ok = true;
        for a in &el.attrs {
            match a.name.as_str() {
                "id" | "name" | "kind" => match a.value.as_str() {
                    Some(s) => match a.name.as_str() {
                        "id" => t.id = Some(s.to_string()),
                        "name" => t.name = s.to_string(),
                        _ => {
                            if !schema::TRACK_TYPES.contains(&s) {
                                self.push(diag(
                                    Severity::Error,
                                    a.value_pos,
                                    "enum.known",
                                    format!("kind must be one of {}, got \"{s}\"", schema::TRACK_TYPES.join(", ")),
                                ));
                                ok = false;
                            }
                            t.kind = s.to_string();
                        }
                    },
                    None => {
                        self.push(diag(
                            Severity::Error,
                            a.value_pos,
                            "value.type",
                            format!("{} must be a string", a.name),
                        ));
                        ok = false;
                    }
                },
                crate::print::EXTRA_ATTR => match a.value.as_object() {
                    Some(o) => {
                        for (k, v) in o {
                            t.fields.insert(k.clone(), v.clone());
                        }
                    }
                    None => {
                        self.push(diag(Severity::Error, a.value_pos, "value.type", "extraFields must be an object"));
                        ok = false;
                    }
                },
                "order" | "type" => {
                    self.push(diag(
                        Severity::Error,
                        a.pos,
                        "attr.reserved",
                        format!(
                            "`{}` is not used in projections: track order is element order and the type is `kind`",
                            a.name
                        ),
                    ));
                    ok = false;
                }
                other => {
                    if let Some(f) = schema::track_field(other) {
                        ok &= self.check_value(f, a);
                    }
                    t.fields.insert(other.to_string(), a.value.clone());
                }
            }
        }
        if t.kind.is_empty() && ok && !el.invalid.iter().any(|a| a == "kind") {
            self.push(diag(
                Severity::Error,
                el.pos,
                "attr.required",
                "<track> needs kind (video, audio, caption, adjustment or layout)",
            ));
            ok = false;
        }
        t.fields = normal::track_fields(&t.fields);
        ok.then_some(t)
    }

    fn item(&mut self, el: &Element, track: usize) -> Option<FileItem> {
        let Some(kind) = kind_for(&el.tag) else {
            self.push(diag(
                Severity::Error,
                el.pos,
                "structure.element",
                format!(
                    "unknown element <{}>; items are clip, image, audio, text, caption, code, adjustment, shader",
                    el.tag
                ),
            ));
            return None;
        };
        let mut ok = true;
        let (mut id, mut start, mut end) = (None, None, None);
        let mut fields = Map::new();
        for a in &el.attrs {
            match a.name.as_str() {
                "id" => match a.value.as_str() {
                    Some(s) => id = Some(s.to_string()),
                    None => {
                        self.push(diag(Severity::Error, a.value_pos, "value.type", "id must be a string"));
                        ok = false;
                    }
                },
                "start" | "end" => {
                    let us = json_to_micros(&a.value).filter(|us| *us >= 0);
                    if us.is_none() {
                        self.push(diag(
                            Severity::Error,
                            a.value_pos,
                            "value.type",
                            format!("{} must be a non-negative number of seconds", a.name),
                        ));
                        ok = false;
                    }
                    if a.name == "start" { start = us } else { end = us }
                }
                "src" => match self.resolve_src(a) {
                    Some(mid) => {
                        fields.insert("mediaItemId".to_string(), Value::String(mid));
                    }
                    None => ok = false,
                },
                crate::print::EXTRA_ATTR => match a.value.as_object() {
                    Some(o) => {
                        for (k, v) in o {
                            fields.insert(k.clone(), v.clone());
                        }
                    }
                    None => {
                        self.push(diag(Severity::Error, a.value_pos, "value.type", "extraFields must be an object"));
                        ok = false;
                    }
                },
                "trackId" | "type" | "startTime" | "endTime" | "trimStart" | "trimEnd" | "mediaItemId" => {
                    let hint = match a.name.as_str() {
                        "trackId" => "the item's track is the <track> it is inside",
                        "type" => "the item's type is its element name",
                        "startTime" => "use start",
                        "endTime" => "use end",
                        "trimStart" => "use in",
                        "trimEnd" => "use out",
                        _ => "use src",
                    };
                    self.push(diag(
                        Severity::Error,
                        a.pos,
                        "attr.reserved",
                        format!("`{}` is not used in projections: {hint}", a.name),
                    ));
                    ok = false;
                }
                other => {
                    let field = field_for(other);
                    if let Some(f) = schema::item_field(kind, field) {
                        ok &= self.check_value(f, a);
                    }
                    fields.insert(
                        field.to_string(),
                        schema::item_field(kind, field)
                            .map(|f| schema::normalise(&f.kind, &a.value))
                            .unwrap_or(a.value.clone()),
                    );
                }
            }
        }
        let (Some(start), Some(end)) = (start, end) else {
            let rejected = el.invalid.iter().any(|a| a == "start" || a == "end");
            if ok && !rejected {
                self.push(diag(Severity::Error, el.pos, "attr.required", format!("<{}> needs start and end", el.tag)));
            }
            return None;
        };
        if !el.children.is_empty() {
            self.push(diag(
                Severity::Error,
                el.children[0].pos,
                "structure.element",
                format!("<{}> cannot contain elements", el.tag),
            ));
            ok = false;
        }
        ok.then(|| FileItem {
            id,
            kind: kind.to_string(),
            track,
            start,
            end,
            fields: normal::item_fields(kind, &fields),
            pos: el.pos,
        })
    }
}

/// Reads the element tree. Returns the model and the diagnostics found (errors make the model unusable).
pub fn read(root: &Element, r: Resolve) -> (FileModel, Vec<Diagnostic>) {
    let mut known_paths = BTreeMap::new();
    if let Some(b) = r.bundle {
        for e in r.current.registry.values() {
            known_paths.insert(path_key(&resolve_path(Some(b), &e.native_path)), e.media_id.clone());
        }
    } else {
        for e in r.current.registry.values() {
            known_paths.insert(path_key(&resolve_path(None, &e.native_path)), e.media_id.clone());
        }
    }
    let mut rd = Reader {
        r,
        diags: Vec::new(),
        base_names: media_names(r.base),
        cur_names: media_names(r.current),
        known_paths,
        new_media: Vec::new(),
    };
    let mut model = FileModel::default();
    if root.tag != "timeline" {
        rd.push(diag(
            Severity::Error,
            root.pos,
            "structure.root",
            format!("the root element must be <timeline>, found <{}>", root.tag),
        ));
        return (model, rd.diags);
    }
    for a in &root.attrs {
        if a.name == "aspect" {
            match a.value.as_str() {
                Some(s) if schema::ASPECT_RATIOS.contains(&s) => model.aspect = Some(s.to_string()),
                _ => rd.push(diag(
                    Severity::Error,
                    a.value_pos,
                    "enum.known",
                    format!("aspect must be one of {}", schema::ASPECT_RATIOS.join(", ")),
                )),
            }
        } else {
            rd.push(diag(
                Severity::Warning,
                a.pos,
                "attr.unknown",
                format!("<timeline> attribute `{}` has no effect", a.name),
            ));
        }
    }
    let mut ids = BTreeMap::new();
    for t in &root.children {
        if t.tag != "track" {
            rd.push(diag(
                Severity::Error,
                t.pos,
                "structure.element",
                format!("<timeline> may only contain <track> elements, found <{}>", t.tag),
            ));
            continue;
        }
        let Some(track) = rd.track(t) else { continue };
        if let Some(id) = &track.id
            && ids.insert(format!("track:{id}"), t.pos).is_some()
        {
            rd.push(diag(Severity::Error, t.pos, "track.id.unique", format!("track id \"{id}\" appears twice")));
        }
        let ti = model.tracks.len();
        model.tracks.push(track);
        for el in &t.children {
            if let Some(item) = rd.item(el, ti) {
                if let Some(id) = &item.id
                    && ids.insert(format!("item:{id}"), el.pos).is_some()
                {
                    rd.push(diag(Severity::Error, el.pos, "item.id.unique", format!("item id \"{id}\" appears twice")));
                    continue;
                }
                if item.start >= item.end {
                    rd.push(diag(Severity::Error, el.pos, "item.time.order", "end must be after start"));
                    continue;
                }
                model.items.push(item);
            }
        }
    }
    model.new_media = rd.new_media;
    (model, rd.diags)
}

/// The media item and registry entry JSON for a new media file (awaiting probe; change 5 probes it).
pub fn new_media_json(m: &NewMedia, now_ms: i64) -> (Value, Value) {
    let file_name = m.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let size = std::fs::metadata(&m.path).map(|md| md.len()).ok();
    let mut entry = json!({"mediaId": m.id, "nativePath": m.path.to_string_lossy(), "ownedByBundle": false, "probeStatus": "awaiting-probe"});
    if let Some(s) = size {
        entry["size"] = json!(s);
    }
    (
        json!({"id": m.id, "createdAt": now_ms, "addedAt": now_ms, "data": {"type": media_type_for(&m.path), "fileName": file_name}}),
        entry,
    )
}
