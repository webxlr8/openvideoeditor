//! Projects in the app: the library (Home), the open project (editor) and every edit the editor makes. All edits go
//! through the document pipeline (`ove-store` → `ove-core` ops), so they are durable, undoable revisions and the
//! bundle's `timeline.tsx` follows them for agents and the CLI.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::media::{MediaState, media_status};
use ove_store::project::{Mode, Project};
use ove_store::{Library, Listing, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};

/// Holder of the library and the project open in the editor (write lock held while open).
#[derive(Default)]
pub struct Projects {
    pub library: Mutex<Option<Library>>,
    pub open: Mutex<Option<Project>>,
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        let code = match e.code() {
            "project.notFound" | "not.found" => ErrorCode::NotFound,
            "project.locked" | "project.readOnly" | "project.newer" => ErrorCode::Unsupported,
            c if c.starts_with("disk") || c.starts_with("permission") || c.starts_with("file") => ErrorCode::Io,
            "io" => ErrorCode::Io,
            _ => ErrorCode::InvalidArgument,
        };
        AppError::new(code, e.to_string())
    }
}

/// A row on Home.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub bundle: String,
    pub id: String,
    pub name: String,
    /// ms since the epoch.
    pub updated_at: f64,
    pub aspect_ratio: String,
    pub duration_seconds: f64,
    pub item_count: u32,
    /// Set when the bundle could not be read.
    pub damaged: Option<String>,
    /// Thumbnail of the project's first video, as a data URL.
    pub thumbnail: Option<String>,
}

fn first_thumbnail(bundle: &Path) -> Option<String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(thumbs_dir(bundle)).ok()?.flatten().map(|e| e.path()).collect();
    files.sort();
    let id = files.first()?.file_stem()?.to_string_lossy().into_owned();
    thumbnail_data_url(bundle, &id)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub locked: bool,
    pub visible: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: String,
    pub track_id: String,
    pub kind: String,
    pub name: String,
    pub start: f64,
    pub end: f64,
    pub trim_start: f64,
    pub media_id: Option<String>,
    pub pending: bool,
    pub hidden: bool,
    pub muted: bool,
    /// Text shown on the clip (text content or caption text).
    pub text: Option<String>,
    /// Transition durations in seconds (0 when none).
    pub transition_in: f64,
    pub transition_out: f64,
    pub speed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub duration_seconds: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// online, offline or changed.
    pub status: String,
    /// `data:image/png;base64,…` thumbnail, when one could be made.
    pub thumbnail: Option<String>,
}

/// Everything the editor shows about the open project.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub bundle: String,
    pub rev: f64,
    pub aspect_ratio: String,
    pub duration_seconds: f64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_summary: Option<String>,
    pub tracks: Vec<TrackView>,
    pub items: Vec<ItemView>,
    pub media: Vec<MediaView>,
}

/// The open project changed (any source).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ProjectChanged(pub Option<ProjectView>);

pub(crate) fn lock<'a, T>(m: &'a Mutex<T>) -> AppResult<std::sync::MutexGuard<'a, T>> {
    m.lock().map_err(|_| AppError::new(ErrorCode::Internal, "internal state is unavailable"))
}

pub(crate) fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

fn secs(us: i64) -> f64 {
    us as f64 / 1e6
}

fn library<'a>(p: &'a Projects) -> AppResult<std::sync::MutexGuard<'a, Option<Library>>> {
    let mut g = lock(&p.library)?;
    if g.is_none() {
        let root = ove_store::library::resolve_library(None)?;
        *g = Some(Library::open(root, clock())?);
    }
    Ok(g)
}

fn thumbs_dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("thumbs")
}

fn thumbnail_data_url(bundle: &Path, media_id: &str) -> Option<String> {
    let bytes = std::fs::read(thumbs_dir(bundle).join(format!("{media_id}.png"))).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn transition_secs(i: &ove_core::Item, key: &str) -> f64 {
    i.fields.get(key).and_then(|t| t.get("duration")).and_then(ove_core::time::json_to_micros).map(secs).unwrap_or(0.0)
}

/// Builds the editor's view of a project.
pub fn view(p: &Project) -> ProjectView {
    let d = p.doc();
    let status = media_status(p.bundle(), d);
    let track_bool = |t: &ove_core::Track, k: &str, def: bool| t.fields.get(k).and_then(Value::as_bool).unwrap_or(def);
    let media = d
        .project
        .media
        .values()
        .map(|m| {
            let data = m.data();
            let get = |k: &str| data.and_then(|x| x.get(k));
            let st = status.iter().find(|s| s.media_id == m.id);
            MediaView {
                id: m.id.clone(),
                name: m.file_name().map(str::to_string).unwrap_or_else(|| m.id.clone()),
                path: st.map(|s| s.path.display().to_string()).unwrap_or_default(),
                kind: get("type").and_then(Value::as_str).unwrap_or("video").to_string(),
                duration_seconds: get("duration").and_then(Value::as_f64),
                width: get("width").and_then(Value::as_u64).map(|v| v as u32),
                height: get("height").and_then(Value::as_u64).map(|v| v as u32),
                status: match st.map(|s| s.state) {
                    Some(MediaState::Offline) => "offline",
                    Some(MediaState::Changed) => "changed",
                    _ => "online",
                }
                .into(),
                thumbnail: thumbnail_data_url(p.bundle(), &m.id),
            }
        })
        .collect();
    ProjectView {
        id: d.project.id.clone(),
        name: d.project.name.clone(),
        bundle: p.bundle().display().to_string(),
        rev: d.project.rev as f64,
        aspect_ratio: d.project.aspect_ratio.clone(),
        duration_seconds: secs(d.duration()),
        can_undo: p.session().can_undo(),
        can_redo: p.session().can_redo(),
        undo_summary: p.session().undo_summary().map(str::to_string),
        tracks: d
            .project
            .tracks
            .iter()
            .map(|t| TrackView {
                id: t.id.clone(),
                name: t.name.clone(),
                kind: t.kind.clone(),
                locked: track_bool(t, "locked", false),
                visible: track_bool(t, "visible", true),
                muted: track_bool(t, "muted", false),
            })
            .collect(),
        items: d
            .items_in_canonical_order()
            .map(|i| ItemView {
                id: i.id.clone(),
                track_id: i.track_id.clone(),
                kind: i.kind.clone(),
                name: if i.name().is_empty() {
                    i.media_id()
                        .and_then(|m| d.project.media.get(m))
                        .and_then(|m| m.file_name().map(str::to_string))
                        .unwrap_or_else(|| i.kind.clone())
                } else {
                    i.name().to_string()
                },
                start: secs(i.start),
                end: secs(i.end),
                trim_start: secs(i.time_field("trimStart")),
                media_id: i.media_id().map(str::to_string),
                pending: i.is_pending(),
                hidden: i.fields.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                muted: i.fields.get("muted").and_then(Value::as_bool).unwrap_or(false),
                text: i
                    .fields
                    .get("textContent")
                    .or_else(|| i.fields.get("captionText"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                transition_in: transition_secs(i, "transitionIn"),
                transition_out: transition_secs(i, "transitionOut"),
                speed: i.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0),
            })
            .collect(),
        media,
    }
}

/// Default tracks of a new project, top layer first.
pub(crate) fn default_tracks() -> Vec<Op> {
    [
        ("t_text", "Text", "video"),
        ("t_captions", "Captions", "caption"),
        ("t_overlay", "Overlay", "video"),
        ("t_main", "Main", "video"),
        ("t_audio", "Audio", "audio"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (id, name, kind))| Op::AddTrack { track: json!({"id": id, "name": name, "order": i, "type": kind}) })
    .collect()
}

pub(crate) fn emit(app: &AppHandle, p: Option<&Project>) {
    let _ = ProjectChanged(p.map(view)).emit(app);
    // The timeline preview follows every change.
    if let Some(p) = p {
        crate::exporting::notify_doc(&app.state::<AppState>(), p.doc(), p.bundle());
    }
}

fn summaries(lib: &Library) -> AppResult<Vec<ProjectSummary>> {
    let mut rows: Vec<ProjectSummary> = lib
        .list()?
        .into_iter()
        .map(|r| match r {
            Listing::Ok { bundle, entry } => ProjectSummary {
                thumbnail: first_thumbnail(&bundle),
                bundle: bundle.display().to_string(),
                id: entry.id,
                name: entry.name,
                updated_at: entry.updated_at as f64,
                aspect_ratio: entry.aspect_ratio,
                duration_seconds: entry.duration,
                item_count: entry.item_count as u32,
                damaged: None,
            },
            Listing::Damaged { bundle, reason } => ProjectSummary {
                name: bundle.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                bundle: bundle.display().to_string(),
                id: String::new(),
                updated_at: 0.0,
                aspect_ratio: String::new(),
                duration_seconds: 0.0,
                item_count: 0,
                damaged: Some(reason),
                thumbnail: None,
            },
        })
        .collect();
    rows.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    Ok(rows)
}

#[tauri::command]
#[specta::specta]
pub fn list_projects(state: State<'_, AppState>) -> AppResult<Vec<ProjectSummary>> {
    let lib = library(&state.projects)?;
    summaries(lib.as_ref().unwrap())
}

fn close_open(state: &AppState) -> AppResult<()> {
    if let Some(p) = lock(&state.projects.open)?.take() {
        p.close()?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn create_project(app: AppHandle, state: State<'_, AppState>, name: String) -> AppResult<ProjectView> {
    close_open(&state)?;
    let mut p = {
        let lib = library(&state.projects)?;
        let name = if name.trim().is_empty() { "Untitled Project".to_string() } else { name };
        lib.as_ref().unwrap().create(&name)?
    };
    p.apply(default_tracks(), Source::Gui, Some("new project".into()))?;
    let v = view(&p);
    tracing::info!(bundle = %p.bundle().display(), "project created");
    *lock(&state.projects.open)? = Some(p);
    emit(&app, lock(&state.projects.open)?.as_ref());
    Ok(v)
}

#[tauri::command]
#[specta::specta]
pub fn open_project(app: AppHandle, state: State<'_, AppState>, bundle: String) -> AppResult<ProjectView> {
    close_open(&state)?;
    let p = Project::open(Path::new(&bundle), Mode::Write, clock())?;
    if let Some(prev) = &p.took_over_from {
        tracing::warn!(previous = %prev.describe(), "recovered a project left open by a process that stopped");
    }
    let v = view(&p);
    *lock(&state.projects.open)? = Some(p);
    emit(&app, lock(&state.projects.open)?.as_ref());
    Ok(v)
}

#[tauri::command]
#[specta::specta]
pub fn close_project(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    close_open(&state)?;
    let _ = state.engine.tx.send(crate::engine::EngineMsg::Close);
    emit(&app, None);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn project_view(state: State<'_, AppState>) -> AppResult<Option<ProjectView>> {
    Ok(lock(&state.projects.open)?.as_ref().map(view))
}

/// Applies ops to the open project as one revision and broadcasts the new view.
pub(crate) fn edit(app: &AppHandle, state: &AppState, ops: Vec<Op>, summary: &str) -> AppResult<ProjectView> {
    let mut g = lock(&state.projects.open)?;
    let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
    if !ops.is_empty() {
        p.apply(ops, Source::Gui, Some(summary.to_string()))?;
    }
    let v = view(p);
    emit(app, Some(p));
    Ok(v)
}

/// Makes a 160×90 PNG thumbnail from the frame at 1 s (or the first frame).
fn make_thumbnail(path: &Path, out: &Path, duration_us: i64) -> Option<()> {
    use ove_media::{DecoderOptions, PixelFormat, VideoDecoder};
    let mut dec =
        ove_media::open_video(path, DecoderOptions { hwaccel: ove_media::HwAccel::Off, ..Default::default() }).ok()?;
    let at = (duration_us / 3).clamp(0, 1_000_000);
    let _ = dec.seek(at);
    let f = dec.next_frame().ok()??;
    let (w, h) = (160u32, 90u32);
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    // Letterbox into 16:9 with nearest sampling.
    let scale = (w as f64 / f.width as f64).min(h as f64 / f.height as f64);
    let (dw, dh) = ((f.width as f64 * scale) as u32, (f.height as f64 * scale) as u32);
    let (ox, oy) = ((w - dw) / 2, (h - dh) / 2);
    for y in 0..dh {
        for x in 0..dw {
            let sx = ((x as f64 / scale) as u32).min(f.width - 1);
            let sy = ((y as f64 / scale) as u32).min(f.height - 1);
            let px = match f.format {
                PixelFormat::Rgba8 => {
                    let p = &f.planes[0];
                    let i = sy as usize * p.stride + sx as usize * 4;
                    [p.data[i], p.data[i + 1], p.data[i + 2]]
                }
                PixelFormat::Nv12 | PixelFormat::P010 => {
                    let deep = f.format == PixelFormat::P010;
                    let (yp, uvp) = (&f.planes[0], &f.planes[1]);
                    let read = |p: &ove_media::Plane, col: usize, row: usize| -> f64 {
                        if deep {
                            let i = row * p.stride + col * 2;
                            (u16::from_le_bytes([p.data[i], p.data[i + 1]]) >> 8) as f64
                        } else {
                            p.data[row * p.stride + col] as f64
                        }
                    };
                    let yy = read(yp, sx as usize, sy as usize) - 16.0;
                    let cu = read(uvp, (sx / 2 * 2) as usize, (sy / 2) as usize) - 128.0;
                    let cv = read(uvp, (sx / 2 * 2 + 1) as usize, (sy / 2) as usize) - 128.0;
                    let c = |v: f64| v.round().clamp(0.0, 255.0) as u8;
                    [c(1.164 * yy + 1.793 * cv), c(1.164 * yy - 0.213 * cu - 0.533 * cv), c(1.164 * yy + 2.112 * cu)]
                }
                PixelFormat::Native(_) => return None,
            };
            let o = (((y + oy) * w + x + ox) * 3) as usize;
            rgb[o..o + 3].copy_from_slice(&px);
        }
    }
    std::fs::create_dir_all(out.parent()?).ok()?;
    let file = std::fs::File::create(out).ok()?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(&rgb).ok()
}

/// Probes files, registers them as media, makes thumbnails and appends each to the Main track (audio-only files to
/// the Audio track) — one revision.
fn import_paths(app: &AppHandle, state: &AppState, paths: Vec<String>) -> AppResult<ProjectView> {
    let (bundle, doc) = {
        let g = lock(&state.projects.open)?;
        let p =
            g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open or create a project first."))?;
        (p.bundle().to_path_buf(), p.doc().clone())
    };
    let known: BTreeSet<String> = doc.registry.values().map(|e| e.native_path.clone()).collect();
    let track_end =
        |track: &str, extra: i64| doc.items_on_track(track).iter().map(|i| i.end).max().unwrap_or(0).max(extra);
    let (mut main_end, mut audio_end) = (track_end("t_main", 0), track_end("t_audio", 0));
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
    let mut ops = Vec::new();
    let mut errors = Vec::new();
    for (n, path) in paths.iter().enumerate() {
        let pb = PathBuf::from(path);
        let canonical = std::fs::canonicalize(&pb).unwrap_or(pb.clone());
        let name = canonical.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if let Err(e) = crate::engine::precheck(&canonical) {
            errors.push(e.message);
            continue;
        }
        let info = match ove_media::probe(&canonical) {
            Ok(i) => i,
            Err(e) => {
                errors.push(format!("“{name}” can't be read: {e}."));
                continue;
            }
        };
        let id = format!("m_{:x}_{n}", now);
        let duration_us = info.duration_us.unwrap_or(5_000_000).max(40_000);
        let mut data = Map::new();
        data.insert("type".into(), (if info.video.is_some() { "video" } else { "audio" }).into());
        data.insert("fileName".into(), name.clone().into());
        data.insert("format".into(), info.container.clone().into());
        data.insert("duration".into(), ove_core::time::micros_to_json(duration_us));
        data.insert("hasAudio".into(), info.audio.is_some().into());
        if let Some(v) = &info.video {
            data.insert("width".into(), v.width.into());
            data.insert("height".into(), v.height.into());
            if let Some(fps) = v.frame_rate.as_f64() {
                data.insert("frameRate".into(), json!((fps * 1000.0).round() / 1000.0));
            }
        }
        let md = std::fs::metadata(&canonical).ok();
        let mut entry = json!({"mediaId": id, "nativePath": canonical.display().to_string(), "ownedByBundle": false, "probeStatus": "probed"});
        if let Some(md) = &md {
            entry["size"] = json!(md.len());
            if let Some(t) = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
                entry["modifiedAt"] = json!(t.as_millis() as i64);
            }
        }
        if known.contains(&canonical.display().to_string()) {
            tracing::info!(path, "media already in the project; adding another clip");
        }
        ops.push(Op::AddMedia {
            media: json!({"id": id, "createdAt": now, "addedAt": now, "data": Value::Object(data)}),
            entry: Some(entry),
        });
        let (track, kind, at) = if info.video.is_some() {
            let at = main_end;
            main_end += duration_us;
            ("t_main", "video", at)
        } else {
            let at = audio_end;
            audio_end += duration_us;
            ("t_audio", "audio", at)
        };
        ops.push(Op::AddItem {
            item: json!({"id": format!("i_{:x}_{n}", now), "trackId": track, "type": kind,
                "startTime": ove_core::time::micros_to_json(at), "endTime": ove_core::time::micros_to_json(at + duration_us),
                "name": name, "mediaItemId": id}),
        });
        if info.video.is_some() {
            let out = thumbs_dir(&bundle).join(format!("{id}.png"));
            if make_thumbnail(&canonical, &out, duration_us).is_none() {
                tracing::warn!(path, "no thumbnail");
            }
        }
    }
    let count = ops.len() / 2;
    let v = edit(app, state, ops, &format!("import {count} file{}", if count == 1 { "" } else { "s" }))?;
    if !errors.is_empty() {
        let _ = crate::AppErrorRaised(AppError::new(ErrorCode::Unsupported, errors.join(" "))).emit(app);
    }
    Ok(v)
}

#[tauri::command]
#[specta::specta]
pub async fn import_media(app: AppHandle, paths: Vec<String>) -> AppResult<ProjectView> {
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || import_paths(&a, &a.state::<AppState>(), paths))
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn import_media_dialog(app: AppHandle) -> AppResult<Option<ProjectView>> {
    let mut exts: Vec<&str> = crate::media::VIDEO_EXTENSIONS.to_vec();
    exts.extend_from_slice(crate::media::AUDIO_EXTENSIONS);
    let Some(files) = app.dialog().file().add_filter("Media", &exts).blocking_pick_files() else { return Ok(None) };
    let paths: Vec<String> =
        files.into_iter().filter_map(|f| f.into_path().ok()).map(|p| p.display().to_string()).collect();
    import_media(app, paths).await.map(Some)
}

/// Moves an item to a new start time (and optionally another track), keeping its duration.
#[tauri::command]
#[specta::specta]
pub fn move_item(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    start: f64,
    track_id: Option<String>,
) -> AppResult<ProjectView> {
    let (old_start, old_end) = {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        let i = p.doc().item(&id).ok_or_else(|| AppError::new(ErrorCode::NotFound, "That clip no longer exists."))?;
        (i.start, i.end)
    };
    let new_start = ove_core::time::secs_to_micros(start.max(0.0)).unwrap_or(old_start);
    let mut set = Map::new();
    set.insert("startTime".into(), ove_core::time::micros_to_json(new_start));
    set.insert("endTime".into(), ove_core::time::micros_to_json(new_start + (old_end - old_start)));
    if let Some(t) = track_id {
        set.insert("trackId".into(), t.into());
    }
    edit(&app, &state, vec![Op::UpdateItem { id, set }], "move clip")
}

#[tauri::command]
#[specta::specta]
pub fn remove_items(app: AppHandle, state: State<'_, AppState>, ids: Vec<String>) -> AppResult<ProjectView> {
    let n = ids.len();
    edit(&app, &state, vec![Op::RemoveItems { ids }], &format!("delete {n} item{}", if n == 1 { "" } else { "s" }))
}

/// Splits a clip at a timeline time (seconds): the left part keeps the id, the right part is a new item.
#[tauri::command]
#[specta::specta]
pub fn split_item(app: AppHandle, state: State<'_, AppState>, id: String, at: f64) -> AppResult<ProjectView> {
    let item = {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        p.doc().item(&id).cloned().ok_or_else(|| AppError::new(ErrorCode::NotFound, "That clip no longer exists."))?
    };
    let cut = ove_core::time::secs_to_micros(at).unwrap_or(0);
    if cut <= item.start || cut >= item.end {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Move the playhead inside the clip to split it."));
    }
    let mut right = item.to_json();
    right["id"] = format!("{}_b{}", item.id, ove_core::Clock::now_ms(&ove_core::SystemClock) % 100_000).into();
    right["startTime"] = ove_core::time::micros_to_json(cut);
    right["trimStart"] = ove_core::time::micros_to_json(item.time_field("trimStart") + (cut - item.start));
    let mut set = Map::new();
    set.insert("endTime".into(), ove_core::time::micros_to_json(cut));
    edit(&app, &state, vec![Op::UpdateItem { id, set }, Op::AddItem { item: right }], "split clip")
}

#[tauri::command]
#[specta::specta]
pub fn set_aspect_ratio(app: AppHandle, state: State<'_, AppState>, aspect: String) -> AppResult<ProjectView> {
    edit(&app, &state, vec![Op::SetAspectRatio { aspect_ratio: aspect }], "change aspect ratio")
}

#[tauri::command]
#[specta::specta]
pub fn rename_project(app: AppHandle, state: State<'_, AppState>, name: String) -> AppResult<ProjectView> {
    let mut set = Map::new();
    set.insert("name".into(), name.into());
    edit(&app, &state, vec![Op::UpdateProject { set, unset: vec![] }], "rename project")
}

#[tauri::command]
#[specta::specta]
pub fn set_track_flag(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    flag: String,
    value: bool,
) -> AppResult<ProjectView> {
    if !matches!(flag.as_str(), "muted" | "visible" | "locked") {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Unknown track setting."));
    }
    let mut set = Map::new();
    set.insert(flag, value.into());
    edit(&app, &state, vec![Op::UpdateTrack { id, set }], "track setting")
}

#[tauri::command]
#[specta::specta]
pub fn undo(app: AppHandle, state: State<'_, AppState>) -> AppResult<ProjectView> {
    let mut g = lock(&state.projects.open)?;
    let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
    p.undo()?;
    emit(&app, Some(p));
    Ok(view(p))
}

#[tauri::command]
#[specta::specta]
pub fn redo(app: AppHandle, state: State<'_, AppState>) -> AppResult<ProjectView> {
    let mut g = lock(&state.projects.open)?;
    let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
    p.redo()?;
    emit(&app, Some(p));
    Ok(view(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_project_has_default_tracks_and_view() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let mut p = lib.create("Trip").unwrap();
        p.apply(default_tracks(), Source::Gui, None).unwrap();
        let v = view(&p);
        assert_eq!(
            v.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["Text", "Captions", "Overlay", "Main", "Audio"]
        );
        assert!(v.can_undo);
        let rows = summaries(&lib).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Trip");
    }
}
