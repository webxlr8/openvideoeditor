//! `organize_media` (add-semantic-media-search D6): folders, tags, renames, subclips and smart folders as document
//! data. One call is one revision through the normal apply pipeline, so it undoes in one step and merges with edits
//! made meanwhile: every change sets single media fields (`name`, `folder`, `tags`, `subclips`) or the project's
//! `folders` list, never a whole media item.

use std::collections::BTreeMap;

use ove_core::model::Document;
use ove_core::ops::Op;
use serde_json::{Map, Value, json};

use crate::ctx::{ToolCtx, edit, via};
use crate::fail::{Fail, plural};
use crate::receipt::Receipt;
use crate::spec::Args;
use crate::tools::media::find_media;

/// The JSON Schema of the `ops` argument.
pub const OPS_SCHEMA: &str = r#"{"type": "array", "minItems": 1, "maxItems": 500, "items": {"type": "object", "required": ["op"], "properties": {"op": {"type": "string", "enum": ["move", "rename", "tag", "createFolder", "renameFolder", "deleteFolder", "saveSubclip", "removeSubclip", "smartFolder"]}, "media": {"type": ["string", "array"], "items": {"type": "string"}}, "folder": {"type": ["string", "null"]}, "name": {"type": "string"}, "add": {"type": "array", "items": {"type": "string"}}, "remove": {"type": "array", "items": {"type": "string"}}, "path": {"type": "string"}, "from": {"type": "string"}, "to": {"type": "string"}, "in": {"type": "number"}, "out": {"type": "number"}, "id": {"type": "string"}, "query": {"type": "string"}, "scope": {"type": "string", "enum": ["all", "moments", "spoken", "timelines", "files"]}}}}"#;

pub const ABOUT: &str = "Organise the project's media in one undoable revision: `move` media into a folder (`folder`: a `/`-separated path, empty or null for the top level), `rename` a media item (display name; `src` names in the timeline follow it), `tag` (`add` / `remove` tags), `createFolder`, `renameFolder` (`from`, `to`; media inside and below move with it), `deleteFolder` (its media move to the parent folder), `saveSubclip` (`media`, `name`, `in`, `out` in source seconds), `removeSubclip` (`media`, `id`) and `smartFolder` (`path`, a saved `query` and `scope`). Ops apply in order; `media` takes ids, file names or timeline names. Edits the person makes meanwhile to other fields are kept.";

/// Normalises a folder path: trims each part, drops empty parts. `None` for the top level.
pub fn folder_path(p: Option<&str>) -> Result<Option<String>, Fail> {
    let Some(p) = p else { return Ok(None) };
    let parts: Vec<&str> = p.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();
    if parts.iter().any(|s| *s == "." || *s == "..") {
        return Err(Fail::new("argument.invalid", format!("folder path {p:?} may not contain . or ..")));
    }
    if parts.iter().any(|s| s.chars().count() > 120) {
        return Err(Fail::new("argument.invalid", "folder names are at most 120 characters"));
    }
    Ok(if parts.is_empty() { None } else { Some(parts.join("/")) })
}

/// True when `path` is `folder` or below it.
fn under(path: &str, folder: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|r| r.starts_with('/'))
}

fn media_list(doc: &Document, bundle: &std::path::Path, v: Option<&Value>) -> Result<Vec<String>, Fail> {
    let names: Vec<String> = match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => return Err(Fail::new("argument.missing", "this op needs `media`")),
    };
    if names.is_empty() {
        return Err(Fail::new("argument.missing", "this op needs `media`"));
    }
    names.iter().map(|q| find_media(doc, bundle, q)).collect()
}

fn text<'a>(o: &'a Map<String, Value>, k: &str, op: &str) -> Result<&'a str, Fail> {
    o.get(k).and_then(Value::as_str).ok_or_else(|| Fail::new("argument.missing", format!("`{op}` needs `{k}`")))
}

fn seconds(o: &Map<String, Value>, k: &str, op: &str) -> Result<f64, Fail> {
    o.get(k)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| Fail::new("argument.invalid", format!("`{op}` needs `{k}` in seconds (0 or more)")))
}

/// The plan of an organise call: the ops to apply (one batch) and what each step did.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub ops: Vec<Op>,
    pub summary: String,
    pub done: Vec<Value>,
}

/// Plans `ops` against `doc`: each step sees the ones before it. Only the media fields that change are set.
pub fn plan(doc: &Document, bundle: &std::path::Path, ops: &[Value]) -> Result<Plan, Fail> {
    // Working copies of the fields this tool owns.
    let mut fields: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let original = |id: &str, k: &str| doc.project.media.get(id).and_then(|m| m.fields.get(k)).cloned();
    let get = |fields: &BTreeMap<String, Map<String, Value>>, id: &str, k: &str| -> Option<Value> {
        match fields.get(id).and_then(|m| m.get(k)) {
            Some(Value::Null) => None,
            Some(v) => Some(v.clone()),
            None => original(id, k),
        }
    };
    let mut folders: Vec<Value> =
        doc.project.extra.get("folders").and_then(Value::as_array).cloned().unwrap_or_default();
    let folders_before = folders.clone();
    let mut done = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (n, raw) in ops.iter().enumerate() {
        let o = raw.as_object().ok_or_else(|| Fail::new("argument.invalid", format!("op {n} is not an object")))?;
        let kind = o.get("op").and_then(Value::as_str).unwrap_or("");
        let at = |e: Fail| Fail::new(e.code, format!("op {n} ({kind}): {}", e.message));
        match kind {
            "move" => {
                let ids = media_list(doc, bundle, o.get("media")).map_err(at)?;
                let folder = folder_path(o.get("folder").and_then(Value::as_str)).map_err(at)?;
                for id in &ids {
                    fields.entry(id.clone()).or_default().insert("folder".into(), folder.clone().into());
                }
                done.push(json!({"op": kind, "media": ids, "folder": folder}));
            }
            "rename" => {
                let ids = media_list(doc, bundle, o.get("media")).map_err(at)?;
                let [id] = ids.as_slice() else {
                    return Err(at(Fail::new("argument.invalid", "rename takes one media item")));
                };
                let name = text(o, "name", kind).map_err(at)?.trim().to_string();
                if name.is_empty() || name.chars().count() > 200 || name.contains(['"', '\n', '\r']) {
                    return Err(at(Fail::new(
                        "argument.invalid",
                        "a name is 1–200 characters without quotes or line breaks",
                    )));
                }
                fields.entry(id.clone()).or_default().insert("name".into(), name.clone().into());
                done.push(json!({"op": kind, "media": id, "name": name}));
            }
            "tag" => {
                let ids = media_list(doc, bundle, o.get("media")).map_err(at)?;
                let list = |k: &str| -> Vec<String> {
                    o.get(k)
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter().filter_map(Value::as_str).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
                        })
                        .into_iter()
                        .flatten()
                        .collect()
                };
                let (add, remove) = (list("add"), list("remove"));
                if add.is_empty() && remove.is_empty() {
                    return Err(at(Fail::new("argument.missing", "tag needs `add` or `remove`")));
                }
                for id in &ids {
                    let mut tags: Vec<String> = get(&fields, id, "tags")
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                    tags.retain(|t| !remove.iter().any(|r| r.eq_ignore_ascii_case(t)));
                    for a in &add {
                        if !tags.iter().any(|t| t.eq_ignore_ascii_case(a)) {
                            tags.push(a.clone());
                        }
                    }
                    let v = if tags.is_empty() { Value::Null } else { json!(tags) };
                    fields.entry(id.clone()).or_default().insert("tags".into(), v);
                }
                done.push(json!({"op": kind, "media": ids, "add": add, "remove": remove}));
            }
            "createFolder" | "smartFolder" => {
                let path = folder_path(Some(text(o, "path", kind).map_err(at)?))
                    .map_err(at)?
                    .ok_or_else(|| at(Fail::new("argument.invalid", "a folder needs a name")))?;
                let mut entry = json!({"path": path});
                if kind == "smartFolder" {
                    let q = text(o, "query", kind).map_err(at)?.trim();
                    if q.is_empty() {
                        return Err(at(Fail::new("argument.invalid", "a smart folder needs a query")));
                    }
                    entry["query"] = q.into();
                    if let Some(s) = o.get("scope").and_then(Value::as_str).filter(|s| *s != "all") {
                        if !ove_core::schema::SEARCH_SCOPES.contains(&s) {
                            return Err(at(Fail::new("argument.invalid", format!("unknown scope {s}"))));
                        }
                        entry["scope"] = s.into();
                    }
                }
                folders.retain(|f| f.get("path").and_then(Value::as_str) != Some(path.as_str()));
                folders.push(entry);
                done.push(json!({"op": kind, "path": path}));
            }
            "renameFolder" => {
                let from = folder_path(Some(text(o, "from", kind).map_err(at)?)).map_err(at)?;
                let to = folder_path(Some(text(o, "to", kind).map_err(at)?)).map_err(at)?;
                let (Some(from), Some(to)) = (from, to) else {
                    return Err(at(Fail::new("argument.invalid", "renameFolder needs two folder paths")));
                };
                if under(&to, &from) && to != from {
                    return Err(at(Fail::new("argument.invalid", "a folder cannot move into itself")));
                }
                let moved = |p: &str| format!("{to}{}", &p[from.len()..]);
                let mut n_media = 0;
                for id in doc.project.media.keys() {
                    if let Some(p) = get(&fields, id, "folder").and_then(|v| v.as_str().map(str::to_string))
                        && under(&p, &from)
                    {
                        fields.entry(id.clone()).or_default().insert("folder".into(), moved(&p).into());
                        n_media += 1;
                    }
                }
                for f in &mut folders {
                    if let Some(p) = f.get("path").and_then(Value::as_str).map(str::to_string)
                        && under(&p, &from)
                    {
                        f["path"] = moved(&p).into();
                    }
                }
                done.push(json!({"op": kind, "from": from, "to": to, "media": n_media}));
            }
            "deleteFolder" => {
                let path = folder_path(Some(text(o, "path", kind).map_err(at)?))
                    .map_err(at)?
                    .ok_or_else(|| at(Fail::new("argument.invalid", "the top level cannot be deleted")))?;
                let parent = path.rsplit_once('/').map(|(p, _)| p.to_string());
                let mut n_media = 0;
                for id in doc.project.media.keys() {
                    if let Some(p) = get(&fields, id, "folder").and_then(|v| v.as_str().map(str::to_string))
                        && under(&p, &path)
                    {
                        fields.entry(id.clone()).or_default().insert("folder".into(), parent.clone().into());
                        n_media += 1;
                    }
                }
                folders.retain(|f| !f.get("path").and_then(Value::as_str).is_some_and(|p| under(p, &path)));
                done.push(json!({"op": kind, "path": path, "media": n_media}));
            }
            "saveSubclip" => {
                let ids = media_list(doc, bundle, o.get("media")).map_err(at)?;
                let [id] = ids.as_slice() else {
                    return Err(at(Fail::new("argument.invalid", "saveSubclip takes one media item")));
                };
                let (a, b) = (seconds(o, "in", kind).map_err(at)?, seconds(o, "out", kind).map_err(at)?);
                if b <= a {
                    return Err(at(Fail::new("argument.invalid", "`out` must be after `in`")));
                }
                let name = o.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
                let mut list: Vec<Value> =
                    get(&fields, id, "subclips").and_then(|v| v.as_array().cloned()).unwrap_or_default();
                let mut k = list.len() + 1;
                let sid = loop {
                    let c = format!("s{k}");
                    if !list.iter().any(|s| s.get("id").and_then(Value::as_str) == Some(c.as_str())) {
                        break c;
                    }
                    k += 1;
                };
                let name = name.map(str::to_string).unwrap_or_else(|| format!("{:.1}–{:.1} s", a, b));
                let us = |s: f64| ove_core::time::micros_to_json((s * 1e6).round() as i64);
                list.push(json!({"id": sid, "name": name, "in": us(a), "out": us(b)}));
                fields.entry(id.clone()).or_default().insert("subclips".into(), json!(list));
                done.push(json!({"op": kind, "media": id, "id": sid, "name": name, "in": a, "out": b}));
            }
            "removeSubclip" => {
                let ids = media_list(doc, bundle, o.get("media")).map_err(at)?;
                let [id] = ids.as_slice() else {
                    return Err(at(Fail::new("argument.invalid", "removeSubclip takes one media item")));
                };
                let sid = text(o, "id", kind).map_err(at)?;
                let mut list: Vec<Value> =
                    get(&fields, id, "subclips").and_then(|v| v.as_array().cloned()).unwrap_or_default();
                let before = list.len();
                list.retain(|s| s.get("id").and_then(Value::as_str) != Some(sid));
                if list.len() == before {
                    return Err(at(Fail::new("subclip.notFound", format!("media {id} has no subclip {sid}"))));
                }
                let v = if list.is_empty() { Value::Null } else { json!(list) };
                fields.entry(id.clone()).or_default().insert("subclips".into(), v);
                done.push(json!({"op": kind, "media": id, "id": sid}));
            }
            other => {
                return Err(Fail::new("argument.invalid", format!("op {n}: unknown op {other:?}")));
            }
        }
        *counts
            .entry(match kind {
                "move" => "moved",
                "rename" => "renamed",
                "tag" => "tagged",
                "saveSubclip" | "removeSubclip" => "subclips",
                _ => "folders",
            })
            .or_default() += 1;
    }
    // One updateMedia per media item, setting or unsetting only fields whose value changes.
    let mut out = Vec::new();
    for (id, f) in fields {
        let (mut set, mut unset) = (Map::new(), Vec::new());
        for (k, v) in f {
            let before = original(&id, &k);
            match v {
                Value::Null if before.is_some() => unset.push(k),
                Value::Null => {}
                v if before.as_ref() != Some(&v) => {
                    set.insert(k, v);
                }
                _ => {}
            }
        }
        if !set.is_empty() || !unset.is_empty() {
            out.push(Op::UpdateMedia { id, set, unset, entry_set: Map::new(), entry_unset: Vec::new() });
        }
    }
    if folders != folders_before {
        if folders.is_empty() {
            out.push(Op::UpdateProject { set: Map::new(), unset: vec!["folders".into()] });
        } else {
            let mut set = Map::new();
            set.insert("folders".into(), json!(folders));
            out.push(Op::UpdateProject { set, unset: Vec::new() });
        }
    }
    let summary =
        format!("organise media: {}", counts.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", "));
    Ok(Plan { ops: out, summary, done })
}

/// `organize_media`: plans the ops and applies them as one revision.
pub fn organize_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let ops = a.value("ops").and_then(Value::as_array).cloned().unwrap_or_default();
    if ops.is_empty() {
        return Err(Fail::new("argument.missing", "`ops` needs at least one op"));
    }
    if ops.len() > 500 {
        return Err(Fail::new("argument.invalid", "at most 500 ops per call"));
    }
    let bundle = ctx.bundle(a.project())?;
    let (rev, p, how) = edit(ctx, &bundle, |w| {
        let p = plan(w.doc(), w.bundle(), &ops)?;
        if p.ops.is_empty() {
            return Ok((w.rev(), p, String::new()));
        }
        let rev = w.apply(p.ops.clone(), p.summary.clone())?;
        Ok((rev, p, via(w).to_string()))
    })?;
    let changed = !p.ops.is_empty();
    let text = if changed {
        format!("rev {rev}: {} ({}){how}", p.summary, plural(p.ops.len(), "change"))
    } else {
        "nothing changed (everything was already organised that way)".to_string()
    };
    Ok(Receipt::new(json!({"rev": rev, "changed": changed, "ops": p.done}), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_paths_are_normalised() {
        assert_eq!(folder_path(Some(" a / b//c ")).unwrap().as_deref(), Some("a/b/c"));
        assert_eq!(folder_path(Some("/")).unwrap(), None);
        assert_eq!(folder_path(None).unwrap(), None);
        assert!(folder_path(Some("a/../b")).is_err());
        assert!(under("a/b", "a") && under("a", "a") && !under("ab", "a"));
    }
}
