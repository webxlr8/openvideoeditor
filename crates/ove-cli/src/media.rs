//! `ove media …`: list, add, remove, relink and probe the files a project uses.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::time::micros_to_json;
use ove_store::media::{MediaState, media_status, resolve_media_path};
use serde_json::{Map, Value, json};

use crate::common::{Ctx, Fail, Out, edit, now_ms, plural, secs, via};

#[derive(Subcommand)]
pub enum MediaCmd {
    /// List the project's media: id, kind, duration, status (online / offline / changed) and how often it is used.
    #[command(alias = "ls")]
    List {
        project: String,
        /// Also media no clip uses.
        #[arg(long)]
        all: bool,
    },
    /// Add files: probes each, registers it and places a clip on the timeline (one revision).
    Add {
        project: String,
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Track (name or id). Default: Main for video and images, Audio for audio.
        #[arg(long)]
        track: Option<String>,
        /// Start of the first clip in seconds (default: the end of the track); the others follow it.
        #[arg(long)]
        at: Option<f64>,
        /// Only register the files; no clips.
        #[arg(long)]
        no_timeline: bool,
        /// Clip length for still images, in seconds.
        #[arg(long, default_value_t = 5.0)]
        image_duration: f64,
    },
    /// Remove media and every clip that uses it.
    #[command(alias = "rm")]
    Remove {
        project: String,
        #[arg(required = true)]
        media: Vec<String>,
    },
    /// Point media at a new file, or find every offline file by name under --folder.
    Relink {
        project: String,
        media: Option<String>,
        path: Option<PathBuf>,
        #[arg(long)]
        folder: Option<PathBuf>,
    },
    /// Container, streams, codecs, duration and frame rate of a media item or any file.
    Probe { project: String, media: String },
    /// Decode one frame to a PNG.
    Grab {
        project: String,
        media: String,
        #[arg(long, default_value_t = 0.0)]
        at: f64,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        out: PathBuf,
    },
    /// A contact sheet of frames across the file (or a range).
    #[command(alias = "sheet")]
    Filmstrip {
        project: String,
        media: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 8)]
        count: u32,
        /// One frame every N seconds (instead of --count).
        #[arg(long)]
        every: Option<f64>,
        #[arg(long, default_value_t = 0.0)]
        from: f64,
        #[arg(long)]
        to: Option<f64>,
        /// Columns (default: up to 5).
        #[arg(long, default_value_t = 0)]
        columns: u32,
        #[arg(long, default_value_t = 240)]
        tile_width: u32,
    },
    /// Hard cuts in a video: times and scores.
    Scenes {
        project: String,
        media: String,
        #[arg(long, default_value_t = 0.3)]
        threshold: f64,
    },
}

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
    let as_path = std::fs::canonicalize(q).unwrap_or_else(|_| PathBuf::from(q));
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

/// The media item or, when `q` is an existing file outside the project, that file.
pub fn media_or_file(ctx: &Ctx, project: &str, q: &str) -> Result<PathBuf, Fail> {
    let p = ctx.read(project)?;
    match find_media(p.doc(), p.bundle(), q) {
        Ok(id) => media_file(p.doc(), p.bundle(), &id),
        Err(e) if Path::new(q).is_file() => {
            let _ = e;
            Ok(PathBuf::from(q))
        }
        Err(e) => Err(e),
    }
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
fn probe_json(path: &Path) -> Result<Value, Fail> {
    let info = ove_media::probe(path).map_err(|e| Fail::new("media.unreadable", format!("{}: {e}", path.display())))?;
    Ok(serde_json::to_value(&info).unwrap_or(Value::Null))
}

#[cfg(not(feature = "media"))]
fn probe_json(_: &Path) -> Result<Value, Fail> {
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
    if let Ok(md) = std::fs::metadata(path) {
        entry["size"] = json!(md.len());
        if let Some(t) = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
            entry["modifiedAt"] = json!(t.as_millis() as i64);
        }
    }
    Ok((json!({"id": id, "createdAt": now, "addedAt": now, "data": Value::Object(data)}), entry, dur, kind))
}

pub fn run(ctx: &Ctx, cmd: &MediaCmd) -> Out {
    match cmd {
        MediaCmd::List { project, all } => {
            let p = ctx.read(project)?;
            let d = p.doc();
            let status: std::collections::BTreeMap<String, MediaState> =
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
            Ok((json!({"media": rows, "count": rows.len()}), text))
        }
        MediaCmd::Add { project, paths, track, at, no_timeline, image_duration } => {
            let mut files = Vec::new();
            for p in paths {
                let c = std::fs::canonicalize(p)
                    .map_err(|e| Fail::new("file.notFound", format!("{}: {e}", p.display())))?;
                if !c.is_file() {
                    return Err(Fail::new("file.notFound", format!("{} is not a file", p.display())));
                }
                files.push(c);
            }
            let image_us = secs(*image_duration, "--image-duration")?.max(40_000);
            let start = at.map(|s| secs(s, "--at")).transpose()?;
            let now = now_ms();
            // Probe before taking the lock: it can take a moment for long files.
            let described: Vec<(PathBuf, Value, Value, i64, &str)> = files
                .iter()
                .enumerate()
                .map(|(n, f)| {
                    describe(f, &format!("m_{now:x}_{n}"), now, image_us).map(|(m, e, d, k)| (f.clone(), m, e, d, k))
                })
                .collect::<Result<_, _>>()?;
            let (rev, added, how, clip_ids, fresh) = edit(ctx, project, |w| {
                let d = w.doc().clone();
                let known: std::collections::BTreeMap<PathBuf, String> = d
                    .registry
                    .values()
                    .map(|e| (resolve_media_path(w.bundle(), &e.native_path), e.media_id.clone()))
                    .collect();
                let mut ops = Vec::new();
                let mut new_tracks: Vec<(String, &str)> = Vec::new();
                let mut cursor: std::collections::BTreeMap<String, i64> = Default::default();
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
                    if *no_timeline {
                        continue;
                    }
                    let track_kind = if *kind == "audio" { "audio" } else { "video" };
                    let tid = match track_for(&d, track.as_deref(), track_kind)? {
                        Some(t) => t,
                        None => {
                            // No track of this kind: create one (video on top of the audio tracks, audio at the bottom).
                            let (id, name) =
                                if track_kind == "audio" { ("t_audio", "Audio") } else { ("t_main", "Main") };
                            if !new_tracks.iter().any(|(i, _)| i == id) {
                                let order =
                                    if track_kind == "audio" { d.project.tracks.len() + new_tracks.len() } else { 0 };
                                ops.insert(
                                    0,
                                    Op::AddTrack {
                                        track: json!({"id": id, "name": name, "order": order, "type": track_kind}),
                                    },
                                );
                                new_tracks.push((id.to_string(), track_kind));
                            }
                            id.to_string()
                        }
                    };
                    let begin = match cursor.get(&tid) {
                        Some(c) => *c,
                        None => {
                            start.unwrap_or_else(|| d.items_on_track(&tid).iter().map(|i| i.end).max().unwrap_or(0))
                        }
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
                let rev = w.apply(ops, ctx.source, format!("add {}", plural(described.len(), "file")))?;
                Ok((rev, added, via(w), clip_ids, fresh))
            })?;
            // Thumbnails for the media panel (video and images).
            if let Ok(b) = ctx.find(project) {
                for (file, id) in &fresh {
                    thumbnail(&b, id, file);
                }
            }
            let v = json!({"rev": rev, "mediaAdded": added, "clips": clip_ids, "files": files});
            Ok((
                v,
                format!("rev {rev}: added {} and {}{how}", plural(added, "media file"), plural(clip_ids.len(), "clip")),
            ))
        }
        MediaCmd::Remove { project, media } => {
            let (rev, items, how) = edit(ctx, project, |w| {
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
                let rev = w.apply(ops, ctx.source, format!("remove {}", plural(ids.len(), "media file")))?;
                Ok((rev, items.len(), via(w)))
            })?;
            Ok((
                json!({"rev": rev, "removedClips": items}),
                format!("rev {rev}: removed {} and {}{how}", plural(media.len(), "media file"), plural(items, "clip")),
            ))
        }
        MediaCmd::Relink { project, media, path, folder } => {
            let (rev, relinked, how) = edit(ctx, project, |w| {
                let d = w.doc();
                let mut pairs: Vec<(String, PathBuf)> = Vec::new();
                match (media, path, folder) {
                    (Some(m), Some(p), None) => {
                        let id = find_media(d, w.bundle(), m)?;
                        let c = std::fs::canonicalize(p)
                            .map_err(|e| Fail::new("file.notFound", format!("{}: {e}", p.display())))?;
                        pairs.push((id, c));
                    }
                    (m, None, Some(dir)) => {
                        let only = m.as_deref().map(|q| find_media(d, w.bundle(), q)).transpose()?;
                        let offline: Vec<_> = media_status(w.bundle(), d)
                            .into_iter()
                            .filter(|s| s.state != MediaState::Online && only.as_ref().is_none_or(|o| *o == s.media_id))
                            .collect();
                        let mut found = std::collections::BTreeMap::new();
                        index_folder(dir, 4, &mut found);
                        for s in offline {
                            let name =
                                s.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
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
                        if let Ok(md) = std::fs::metadata(p) {
                            entry_set.insert("size".into(), md.len().into());
                            if let Some(t) =
                                md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            {
                                entry_set.insert("modifiedAt".into(), (t.as_millis() as i64).into());
                            }
                        }
                        Op::UpdateMedia {
                            id: id.clone(),
                            set: Map::new(),
                            unset: vec![],
                            entry_set,
                            entry_unset: vec![],
                        }
                    })
                    .collect();
                let rev = w.apply(ops, ctx.source, format!("relink {}", plural(pairs.len(), "media file")))?;
                Ok((rev, pairs, via(w)))
            })?;
            let v = json!({"rev": rev, "relinked": relinked.iter().map(|(id, p)| json!({"id": id, "path": p})).collect::<Vec<_>>()});
            let text = if relinked.is_empty() {
                "nothing to relink".to_string()
            } else {
                format!("rev {rev}: relinked {}{how}", plural(relinked.len(), "media file"))
            };
            Ok((v, text))
        }
        MediaCmd::Grab { project, media, at, width, out } => {
            crate::analyze::grab(ctx, project, media, *at, *width, out)
        }
        MediaCmd::Filmstrip { project, media, out, count, every, from, to, columns, tile_width } => {
            crate::analyze::filmstrip(
                ctx,
                project,
                media,
                &crate::analyze::StripOpts {
                    count: *count,
                    every: *every,
                    from: *from,
                    to: *to,
                    columns: *columns,
                    tile_width: *tile_width,
                },
                out,
            )
        }
        MediaCmd::Scenes { project, media, threshold } => crate::analyze::scenes(ctx, project, media, *threshold),
        MediaCmd::Probe { project, media } => {
            let file = media_or_file(ctx, project, media)?;
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
            if let Some(a) = v.get("audio").filter(|x| !x.is_null()) {
                text.push_str(&format!(
                    "\naudio {} {} Hz {}",
                    a["codec"].as_str().unwrap_or("?"),
                    a["sampleRate"],
                    a["layout"].as_str().unwrap_or("")
                ));
            }
            Ok((v, text))
        }
    }
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
            .and_then(|_| std::fs::write(dir.join(format!("{media_id}.png")), canvas.png()));
    }
}

#[cfg(not(feature = "media"))]
pub fn thumbnail(_: &Path, _: &str, _: &Path) {}

/// File name (lower-case) → path for files under `dir`, `depth` levels deep; first match wins.
fn index_folder(dir: &Path, depth: u32, out: &mut std::collections::BTreeMap<String, PathBuf>) {
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
