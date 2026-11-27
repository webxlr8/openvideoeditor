//! The files a project uses: `list_media`, `add_media`, `remove_media`, `relink_media`, `probe_media`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::time::micros_to_json;
use ove_store::media::{MediaState, media_status, resolve_media_path};
use ove_store::project::Project;
use serde_json::{Map, Value, json};

use crate::ctx::{ToolCtx, edit, read, via};
use crate::fail::{Fail, now_ms, plural, secs};
use crate::receipt::Receipt;
use crate::spec::Args;

const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];
const AUDIO_EXTENSIONS: [&str; 11] = ["m4a", "wav", "caf", "opus", "ogg", "aac", "mp3", "flac", "aif", "aiff", "wma"];

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// A media item by id, file name (case-insensitive) or path.
pub fn find_media(doc: &Document, bundle: &Path, q: &str) -> Result<String, Fail> {
    if doc.project.media.contains_key(q) {
        return Ok(q.to_string());
    }
    let ql = q.to_lowercase();
    let by_name: Vec<&String> = doc
        .project
        .media
        .values()
        .filter(|m| m.file_name().is_some_and(|n| n.to_lowercase() == ql))
        .map(|m| &m.id)
        .collect();
    if by_name.len() == 1 {
        return Ok(by_name[0].clone());
    }
    if by_name.len() > 1 {
        return Err(Fail::new("media.ambiguous", format!("{} media items are named {q}; use the id", by_name.len())));
    }
    let as_path = dunce::canonicalize(q).unwrap_or_else(|_| PathBuf::from(q));
    if let Some(e) = doc.registry.values().find(|e| resolve_media_path(bundle, &e.native_path) == as_path) {
        return Ok(e.media_id.clone());
    }
    Err(Fail::new("media.notFound", format!("no media \"{q}\" in this project (see `ove media ls`)")))
}

/// The file behind a media item.
pub fn media_file(doc: &Document, bundle: &Path, id: &str) -> Result<PathBuf, Fail> {
    let e = doc.registry.get(id).ok_or_else(|| Fail::new("media.unregistered", format!("media {id} has no file")))?;
    Ok(resolve_media_path(bundle, &e.native_path))
}

/// The file of a media item of the call's project or, when `media` is an existing file outside the project, that
/// file.
pub fn media_or_file(ctx: &mut dyn ToolCtx, a: &Args) -> Result<PathBuf, Fail> {
    let q = a.req_str("media")?.to_string();
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| match find_media(p.doc(), p.bundle(), &q) {
        Ok(id) => media_file(p.doc(), p.bundle(), &id),
        Err(_) if Path::new(&q).is_file() => Ok(PathBuf::from(&q)),
        Err(e) => Err(e),
    })
}

fn track_for(doc: &Document, wanted: Option<&str>, kind: &str) -> Result<Option<String>, Fail> {
    if let Some(q) = wanted {
        let t = doc
            .project
            .tracks
            .iter()
            .find(|t| t.id == q)
            .or_else(|| doc.project.tracks.iter().find(|t| t.name.eq_ignore_ascii_case(q)))
            .ok_or_else(|| Fail::new("track.notFound", format!("no track \"{q}\"")))?;
        let audio = kind == "audio";
        if (t.kind == "audio") != audio || t.kind == "caption" {
            return Err(Fail::new("track.kind", format!("track \"{}\" holds {} clips, not {kind}", t.name, t.kind)));
        }
        return Ok(Some(t.id.clone()));
    }
    let (preferred, track_kind) = if kind == "audio" { ("t_audio", "audio") } else { ("t_main", "video") };
    if doc.track(preferred).is_some() {
        return Ok(Some(preferred.into()));
    }
    // The lowest track of the kind.
    Ok(doc.project.tracks.iter().rev().find(|t| t.kind == track_kind).map(|t| t.id.clone()))
}

#[cfg(feature = "media")]
pub fn probe_json(path: &Path) -> Result<Value, Fail> {
    let info = ove_media::probe(path).map_err(|e| Fail::new("media.unreadable", format!("{}: {e}", path.display())))?;
    Ok(serde_json::to_value(&info).unwrap_or(Value::Null))
}

#[cfg(not(feature = "media"))]
pub fn probe_json(_: &Path) -> Result<Value, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

/// Media record + registry entry + clip duration (µs) + kind for a file.
fn describe(path: &Path, id: &str, now: i64, image_us: i64) -> Result<(Value, Value, i64, &'static str), Fail> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let e = ext(path);
    let mut data = Map::new();
    data.insert("fileName".into(), name.into());
    let (kind, dur) = if IMAGE_EXTENSIONS.contains(&e.as_str()) {
        let info = probe_json(path).ok();
        if let Some(v) = info.as_ref().and_then(|i| i.get("video")) {
            data.insert("width".into(), v["width"].clone());
            data.insert("height".into(), v["height"].clone());
        }
        ("image", image_us)
    } else {
        let info = probe_json(path)?;
        let has_video = info.get("video").is_some_and(|v| !v.is_null());
        let has_audio = info.get("audio").is_some_and(|v| !v.is_null());
        if !has_video && !has_audio {
            return Err(Fail::new("media.unsupported", format!("{} has no audio or video", path.display())));
        }
        let dur = info.get("durationUs").and_then(Value::as_i64).unwrap_or(5_000_000).max(40_000);
        data.insert("format".into(), info["container"].clone());
        data.insert("duration".into(), micros_to_json(dur));
        data.insert("hasAudio".into(), has_audio.into());
        if has_video {
            let v = &info["video"];
            data.insert("width".into(), v["width"].clone());
            data.insert("height".into(), v["height"].clone());
            if let (Some(n), Some(d)) = (
                v.pointer("/frameRate/num").and_then(Value::as_f64),
                v.pointer("/frameRate/den").and_then(Value::as_f64),
            ) && d > 0.0
            {
                data.insert("frameRate".into(), json!(((n / d) * 1000.0).round() / 1000.0));
            }
        }
        let kind = if has_video && !AUDIO_EXTENSIONS.contains(&e.as_str()) { "video" } else { "audio" };
        (kind, dur)
    };
    data.insert("type".into(), kind.into());
    let mut entry = json!({"mediaId": id, "nativePath": path.display().to_string(), "ownedByBundle": false, "probeStatus": "probed"});
    // size, modifiedAt and contentId (derived data is keyed by the content id).
    if let Some(e) = entry.as_object_mut() {
        e.extend(ove_store::media::identity_fields(path));
    }
    Ok((json!({"id": id, "createdAt": now, "addedAt": now, "data": Value::Object(data)}), entry, dur, kind))
}

/// The project's media as a receipt (`all`: also media no clip uses).
pub fn list_receipt(p: &Project, all: bool) -> Receipt {
    let d = p.doc();
    let status: BTreeMap<String, MediaState> =
        media_status(p.bundle(), d).into_iter().map(|m| (m.media_id, m.state)).collect();
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for m in d.project.media.values() {
        let uses = d.project.items.values().filter(|i| i.media_id() == Some(m.id.as_str())).count();
        if uses == 0 && !all {
            continue;
        }
        let data = m.data().cloned().unwrap_or_default();
        let kind = data.get("type").and_then(Value::as_str).unwrap_or("?").to_string();
        let dur = data.get("duration").and_then(Value::as_f64);
        let st = status.get(&m.id).copied();
        let path = d.registry.get(&m.id).map(|e| resolve_media_path(p.bundle(), &e.native_path));
        let st_s = match st {
            Some(MediaState::Online) => "online",
            Some(MediaState::Offline) => "offline",
            Some(MediaState::Changed) => "changed",
            None => "unregistered",
        };
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            m.id,
            kind,
            dur.map(|s| format!("{s:.2}s")).unwrap_or_else(|| "-".into()),
            st_s,
            plural(uses, "use"),
            m.file_name().unwrap_or("")
        ));
        rows.push(json!({"id": m.id, "name": m.file_name(), "kind": kind, "durationSeconds": dur, "status": st_s, "uses": uses, "path": path, "data": data}));
    }
    let text = if lines.is_empty() { "no media".into() } else { lines.join("\n") };
    Receipt::new(json!({"media": rows, "count": rows.len()}), text)
}

pub fn list_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let all = a.bool("all");
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| Ok(list_receipt(p, all)))
}

pub fn add_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let mut files = Vec::new();
    for p in a.strings("paths") {
        let c = dunce::canonicalize(&p).map_err(|e| Fail::new("file.notFound", format!("{p}: {e}")))?;
        if !c.is_file() {
            return Err(Fail::new("file.notFound", format!("{p} is not a file")));
        }
        files.push(c);
    }
    if files.is_empty() {
        return Err(Fail::new("argument.missing", "`paths` must name at least one file"));
    }
    let image_us = secs(a.f64("imageDuration").unwrap_or(5.0), "--image-duration")?.max(40_000);
    let start = a.f64("at").map(|s| secs(s, "--at")).transpose()?;
    let track = a.str("track").map(str::to_string);
    let no_timeline = a.bool("noTimeline");
    let now = now_ms();
    // Probe before taking the project: it can take a moment for long files.
    let described: Vec<(PathBuf, Value, Value, i64, &str)> = files
        .iter()
        .enumerate()
        .map(|(n, f)| describe(f, &format!("m_{now:x}_{n}"), now, image_us).map(|(m, e, d, k)| (f.clone(), m, e, d, k)))
        .collect::<Result<_, _>>()?;
    let bundle = ctx.bundle(a.project())?;
    let (rev, added, how, clip_ids, fresh) = edit(ctx, &bundle, |w| {
        let d = w.doc().clone();
        let known: BTreeMap<PathBuf, String> =
            d.registry.values().map(|e| (resolve_media_path(w.bundle(), &e.native_path), e.media_id.clone())).collect();
        let mut ops = Vec::new();
        let mut new_tracks: Vec<(String, &str)> = Vec::new();
        let mut cursor: BTreeMap<String, i64> = Default::default();
        let mut clip_ids = Vec::new();
        let mut fresh: Vec<(PathBuf, String)> = Vec::new();
        for (n, (file, media, entry, dur, kind)) in described.iter().enumerate() {
            let media_id = match known.get(file) {
                Some(id) => id.clone(),
                None => {
                    if *kind != "audio" {
                        fresh.push((file.clone(), media["id"].as_str().unwrap_or_default().to_string()));
                    }
                    ops.push(Op::AddMedia { media: media.clone(), entry: Some(entry.clone()) });
                    media["id"].as_str().unwrap_or_default().to_string()
                }
            };
            if no_timeline {
                continue;
            }
            let track_kind = if *kind == "audio" { "audio" } else { "video" };
            let tid = match track_for(&d, track.as_deref(), track_kind)? {
                Some(t) => t,
                None => {
                    // No track of this kind: create one (video on top of the audio tracks, audio at the bottom).
                    let (id, name) = if track_kind == "audio" { ("t_audio", "Audio") } else { ("t_main", "Main") };
                    if !new_tracks.iter().any(|(i, _)| i == id) {
                        let order = if track_kind == "audio" { d.project.tracks.len() + new_tracks.len() } else { 0 };
                        ops.insert(
                            0,
                            Op::AddTrack { track: json!({"id": id, "name": name, "order": order, "type": track_kind}) },
                        );
                        new_tracks.push((id.to_string(), track_kind));
                    }
                    id.to_string()
                }
            };
            let begin = match cursor.get(&tid) {
                Some(c) => *c,
                None => start.unwrap_or_else(|| d.items_on_track(&tid).iter().map(|i| i.end).max().unwrap_or(0)),
            };
            cursor.insert(tid.clone(), begin + dur);
            let id = format!("i_{now:x}_{n}");
            let name = file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            ops.push(Op::AddItem {
                item: json!({"id": id, "trackId": tid, "type": kind, "startTime": micros_to_json(begin),
                    "endTime": micros_to_json(begin + dur), "name": name, "mediaItemId": media_id}),
            });
            clip_ids.push(id);
        }
        let added = ops.iter().filter(|o| matches!(o, Op::AddMedia { .. })).count();
        if ops.is_empty() {
            return Ok((w.rev(), 0, via(w), clip_ids, fresh));
        }
        let rev = w.apply(ops, format!("add {}", plural(described.len(), "file")))?;
        Ok((rev, added, via(w), clip_ids, fresh))
    })?;
    // Thumbnails for the media panel (video and images).
    for (file, id) in &fresh {
        thumbnail(&bundle, id, file);
    }
    let v = json!({"rev": rev, "mediaAdded": added, "clips": clip_ids, "files": files});
    Ok(Receipt::new(
        v,
        format!("rev {rev}: added {} and {}{how}", plural(added, "media file"), plural(clip_ids.len(), "clip")),
    ))
}

pub fn remove_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let media = a.strings("media");
    let bundle = ctx.bundle(a.project())?;
    let (rev, items, how) = edit(ctx, &bundle, |w| {
        let d = w.doc();
        let ids: Vec<String> = media.iter().map(|q| find_media(d, w.bundle(), q)).collect::<Result<_, _>>()?;
        let items: Vec<String> = d
            .project
            .items
            .values()
            .filter(|i| i.media_id().is_some_and(|m| ids.iter().any(|x| x == m)))
            .map(|i| i.id.clone())
            .collect();
        let mut ops = Vec::new();
        if !items.is_empty() {
            ops.push(Op::RemoveItems { ids: items.clone() });
        }
        ops.extend(ids.iter().map(|id| Op::RemoveMedia { id: id.clone() }));
        let rev = w.apply(ops, format!("remove {}", plural(ids.len(), "media file")))?;
        Ok((rev, items.len(), via(w)))
    })?;
    Ok(Receipt::new(
        json!({"rev": rev, "removedClips": items}),
        format!("rev {rev}: removed {} and {}{how}", plural(media.len(), "media file"), plural(items, "clip")),
    ))
}

pub fn relink_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let media = a.str("media").map(str::to_string);
    let path = a.str("path").map(PathBuf::from);
    let folder = a.str("folder").map(PathBuf::from);
    let bundle = ctx.bundle(a.project())?;
    let (rev, relinked, how) = edit(ctx, &bundle, |w| {
        let d = w.doc();
        let mut pairs: Vec<(String, PathBuf)> = Vec::new();
        match (&media, &path, &folder) {
            (Some(m), Some(p), None) => {
                let id = find_media(d, w.bundle(), m)?;
                let c =
                    dunce::canonicalize(p).map_err(|e| Fail::new("file.notFound", format!("{}: {e}", p.display())))?;
                pairs.push((id, c));
            }
            (m, None, Some(dir)) => {
                let only = m.as_deref().map(|q| find_media(d, w.bundle(), q)).transpose()?;
                let offline: Vec<_> = media_status(w.bundle(), d)
                    .into_iter()
                    .filter(|s| s.state != MediaState::Online && only.as_ref().is_none_or(|o| *o == s.media_id))
                    .collect();
                let mut found = BTreeMap::new();
                index_folder(dir, 4, &mut found);
                for s in offline {
                    let name = s.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if let Some(p) = found.get(&name) {
                        pairs.push((s.media_id, p.clone()));
                    }
                }
            }
            _ => {
                return Err(Fail::new(
                    "argument.invalid",
                    "give MEDIA and PATH, or --folder DIR (optionally with MEDIA)",
                ));
            }
        }
        if pairs.is_empty() {
            return Ok((w.rev(), vec![], via(w)));
        }
        let ops = pairs
            .iter()
            .map(|(id, p)| {
                let mut entry_set = Map::new();
                entry_set.insert("nativePath".into(), p.display().to_string().into());
                entry_set.insert("ownedByBundle".into(), false.into());
                // The relinked file's identity: the same content keeps its derived data.
                entry_set.extend(ove_store::media::identity_fields(p));
                Op::UpdateMedia { id: id.clone(), set: Map::new(), unset: vec![], entry_set, entry_unset: vec![] }
            })
            .collect();
        let rev = w.apply(ops, format!("relink {}", plural(pairs.len(), "media file")))?;
        Ok((rev, pairs, via(w)))
    })?;
    let v = json!({"rev": rev, "relinked": relinked.iter().map(|(id, p)| json!({"id": id, "path": p})).collect::<Vec<_>>()});
    let text = if relinked.is_empty() {
        "nothing to relink".to_string()
    } else {
        format!("rev {rev}: relinked {}{how}", plural(relinked.len(), "media file"))
    };
    Ok(Receipt::new(v, text))
}

pub fn probe_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let file = media_or_file(ctx, a)?;
    let v = probe_json(&file)?;
    let mut text = format!("{}\ncontainer {}", file.display(), v["container"].as_str().unwrap_or("?"));
    if let Some(d) = v["durationUs"].as_i64() {
        text.push_str(&format!(" · {:.3} s", d as f64 / 1e6));
    }
    if let Some(vs) = v.get("video").filter(|x| !x.is_null()) {
        text.push_str(&format!(
            "\nvideo {} {}x{} {}/{} fps{}",
            vs["codec"].as_str().unwrap_or("?"),
            vs["width"],
            vs["height"],
            vs.pointer("/frameRate/num").unwrap_or(&Value::Null),
            vs.pointer("/frameRate/den").unwrap_or(&Value::Null),
            if vs["variableFrameRate"].as_bool() == Some(true) { " (variable)" } else { "" }
        ));
    }
    if let Some(au) = v.get("audio").filter(|x| !x.is_null()) {
        text.push_str(&format!(
            "\naudio {} {} Hz {}",
            au["codec"].as_str().unwrap_or("?"),
            au["sampleRate"],
            au["layout"].as_str().unwrap_or("")
        ));
    }
    Ok(Receipt::new(v, text))
}

/// The thumbnail the app's media panel shows (`cache/thumbs/<media id>.png`, 160×90, letterboxed).
#[cfg(feature = "media")]
pub fn thumbnail(bundle: &Path, media_id: &str, file: &Path) {
    // The id names the file; ids come from the project document.
    if !ove_guard::is_safe_id(media_id) {
        return;
    }
    if let Ok(img) = ove_analyze::grab(file, 0.5, Some(160)) {
        let mut canvas = ove_analyze::Image::filled(160, 90, [0, 0, 0, 255]);
        let scale = (160.0 / img.width as f64).min(90.0 / img.height as f64);
        let fit = img.resize(((img.width as f64 * scale) as u32).max(1), ((img.height as f64 * scale) as u32).max(1));
        canvas.blit(&fit, (160 - fit.width) / 2, (90 - fit.height) / 2);
        let dir = bundle.join("cache").join("thumbs");
        let _ = std::fs::create_dir_all(&dir)
            .and_then(|_| canvas.png())
            .and_then(|png| std::fs::write(dir.join(format!("{media_id}.png")), png));
    }
}

#[cfg(not(feature = "media"))]
pub fn thumbnail(_: &Path, _: &str, _: &Path) {}

/// File name (lower-case) → path for files under `dir`, `depth` levels deep; first match wins.
fn index_folder(dir: &Path, depth: u32, out: &mut BTreeMap<String, PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 && !e.file_name().to_string_lossy().starts_with('.') {
                index_folder(&p, depth - 1, out);
            }
        } else if let Some(n) = p.file_name() {
            out.entry(n.to_string_lossy().to_lowercase()).or_insert(p);
        }
    }
}
