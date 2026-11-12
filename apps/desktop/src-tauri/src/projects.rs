//! Projects in the app: the library (Home), the open project (editor) and every edit the editor makes. All edits go
//! through the document pipeline (`ove-store` → `ove-core` ops), so they are durable, undoable revisions and the
//! bundle's `timeline.tsx` follows them for agents and the CLI.
//!
//! Commands that read or change the open project run off the main thread, one at a time and in the order they
//! arrive ([`serial`]), so a slow disk never freezes the window and quick edits (a slider drag) never reorder.
//! Building the editor's view never touches the disk: media status and thumbnails come from [`MediaCache`], which a
//! background check ([`refresh_media`]) fills when the project opens, its media change, or the window regains focus.

use std::any::Any;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::media::{MediaState, media_status, resolve_media_path};
use ove_store::project::{Mode, Project};
use ove_store::{Library, Listing, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker::{self, relock};

/// Holder of the library and the project open in the editor (write lock held while open).
#[derive(Default)]
pub struct Projects {
    pub library: Mutex<Option<Library>>,
    pub open: Mutex<Option<Project>>,
    /// Media status and thumbnails of the open project.
    pub media: Mutex<MediaCache>,
    /// Project commands take turns here, first come first served.
    turn: tokio::sync::Mutex<()>,
    refresh: RefreshState,
}

#[derive(Default)]
struct RefreshState {
    running: AtomicBool,
    again: AtomicBool,
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

/// Locks shared state. A lock that a panicking thread left poisoned is recovered instead of failing every later
/// command: the poison is cleared, and the open project is re-validated (see [`heal_project`]).
pub(crate) fn lock<T: 'static>(m: &Mutex<T>) -> AppResult<MutexGuard<'_, T>> {
    match m.lock() {
        Ok(g) => Ok(g),
        Err(poisoned) => {
            m.clear_poison();
            let mut g = poisoned.into_inner();
            tracing::warn!(state = std::any::type_name::<T>(), "recovering state left locked by a failed command");
            if let Some(open) = (&mut *g as &mut dyn Any).downcast_mut::<Option<Project>>() {
                heal_project(open);
            }
            Ok(g)
        }
    }
}

/// Re-validates the open project after a command panicked while holding it. The revision log on disk is the
/// source of truth: a document that fails its invariants is reloaded from it (undo history before the failure is
/// lost); one that cannot be reloaded is closed.
fn heal_project(open: &mut Option<Project>) {
    let Some(p) = open.as_ref() else { return };
    match ove_core::migrate::validate(p.doc()) {
        Ok(()) => tracing::info!(rev = p.rev(), "the open project is consistent after the failure; keeping it"),
        Err(e) => {
            let bundle = p.bundle().to_path_buf();
            tracing::error!(error = %e, "the open project is inconsistent after the failure; reloading it from disk");
            // Dropping it releases the write lock so it can be taken again.
            *open = None;
            match Project::open(&bundle, Mode::Write, clock()) {
                Ok(p) => *open = Some(p),
                Err(e) => tracing::error!(error = %e, "reloading failed; the project was closed"),
            }
        }
    }
}

pub(crate) fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

fn secs(us: i64) -> f64 {
    us as f64 / 1e6
}

fn library<'a>(p: &'a Projects) -> AppResult<MutexGuard<'a, Option<Library>>> {
    let mut g = lock(&p.library)?;
    if g.is_none() {
        let root = ove_store::library::resolve_library(None)?;
        *g = Some(Library::open(root, clock())?);
    }
    Ok(g)
}

pub(crate) fn thumbs_dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("thumbs")
}

fn thumbnail_data_url(bundle: &Path, media_id: &str) -> Option<String> {
    let bytes = std::fs::read(thumbs_dir(bundle).join(format!("{media_id}.png"))).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn transition_secs(i: &ove_core::Item, key: &str) -> f64 {
    i.fields.get(key).and_then(|t| t.get("duration")).and_then(ove_core::time::json_to_micros).map(secs).unwrap_or(0.0)
}

/// What the view shows about one media file's link to disk.
#[derive(Debug, Clone, PartialEq)]
struct LinkView {
    path: String,
    state: MediaState,
}

fn status_name(s: MediaState) -> &'static str {
    match s {
        MediaState::Offline => "offline",
        MediaState::Changed => "changed",
        MediaState::Online => "online",
    }
}

/// Builds the editor's view of a project, checking every media file and reading every thumbnail now. Commands use
/// the cached view instead ([`MediaCache::view`]); this is for one-off use and tests.
pub fn view(p: &Project) -> ProjectView {
    let status = media_status(p.bundle(), p.doc());
    view_from(
        p,
        |id| {
            status
                .iter()
                .find(|s| s.media_id == id)
                .map(|s| LinkView { path: s.path.display().to_string(), state: s.state })
        },
        |id| thumbnail_data_url(p.bundle(), id),
    )
}

fn view_from(
    p: &Project,
    link: impl Fn(&str) -> Option<LinkView>,
    thumbnail: impl Fn(&str) -> Option<String>,
) -> ProjectView {
    let d = p.doc();
    let track_bool = |t: &ove_core::Track, k: &str, def: bool| t.fields.get(k).and_then(Value::as_bool).unwrap_or(def);
    let media = d
        .project
        .media
        .values()
        .map(|m| {
            let data = m.data();
            let get = |k: &str| data.and_then(|x| x.get(k));
            let st = link(&m.id);
            MediaView {
                id: m.id.clone(),
                name: m.file_name().map(str::to_string).unwrap_or_else(|| m.id.clone()),
                path: st.as_ref().map(|s| s.path.clone()).unwrap_or_default(),
                kind: get("type").and_then(Value::as_str).unwrap_or("video").to_string(),
                duration_seconds: get("duration").and_then(Value::as_f64),
                width: get("width").and_then(Value::as_u64).map(|v| v as u32),
                height: get("height").and_then(Value::as_u64).map(|v| v as u32),
                status: status_name(st.map(|s| s.state).unwrap_or(MediaState::Online)).into(),
                thumbnail: thumbnail(&m.id),
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

/// The registry fields a status check depends on: when they change (import, relink), the file is checked again.
fn link_key(e: &ove_core::RegistryEntry) -> String {
    format!("{}\u{1}{:?}\u{1}{:?}", e.native_path, e.fields.get("size"), e.fields.get("modifiedAt"))
}

/// Media status and thumbnails of the open project as last checked, so building a view (on every edit) does no
/// file-system access: no `stat` of each media file, no read and base64 of each thumbnail.
#[derive(Default)]
pub struct MediaCache {
    bundle: PathBuf,
    /// media id → (link key it was checked for, result).
    links: HashMap<String, (String, LinkView)>,
    /// media id → thumbnail data URL (`None`: there was no thumbnail file when last looked).
    thumbs: HashMap<String, Option<String>>,
}

impl MediaCache {
    fn select(&mut self, bundle: &Path) {
        if self.bundle != bundle {
            *self = MediaCache { bundle: bundle.to_path_buf(), ..Default::default() };
        }
    }

    /// The view of `p` from the cache, and whether some media still needs checking (see [`refresh_media`]).
    /// Unchecked media shows as online at its registered path until the check reports otherwise.
    pub fn view(&mut self, p: &Project) -> (ProjectView, bool) {
        self.select(p.bundle());
        let d = p.doc();
        let stale = d.registry.values().any(|e| self.links.get(&e.media_id).is_none_or(|(k, _)| *k != link_key(e)))
            || d.project.media.keys().any(|id| !self.thumbs.contains_key(id));
        let v = view_from(
            p,
            |id| {
                let e = d.registry.get(id)?;
                Some(match self.links.get(id) {
                    Some((k, l)) if *k == link_key(e) => l.clone(),
                    _ => LinkView {
                        path: resolve_media_path(p.bundle(), &e.native_path).display().to_string(),
                        state: MediaState::Online,
                    },
                })
            },
            |id| self.thumbs.get(id).cloned().flatten(),
        );
        (v, stale)
    }

    /// Records a thumbnail just made for `media_id`.
    pub fn put_thumbnail(&mut self, bundle: &Path, media_id: &str, data_url: Option<String>) {
        self.select(bundle);
        self.thumbs.insert(media_id.to_string(), data_url);
    }

    /// Stores checked links and thumbnails; true when something the view shows changed.
    fn store(
        &mut self,
        bundle: &Path,
        links: Vec<(String, String, LinkView)>,
        thumbs: Vec<(String, Option<String>)>,
    ) -> bool {
        self.select(bundle);
        let mut changed = false;
        for (id, key, l) in links {
            let shown = match self.links.get(&id) {
                Some((k, old)) if *k == key => old.clone(),
                // Shown as online at its path until now.
                _ => LinkView { path: l.path.clone(), state: MediaState::Online },
            };
            changed |= shown != l;
            self.links.insert(id, (key, l));
        }
        for (id, t) in thumbs {
            changed |= self.thumbs.get(&id).cloned().flatten() != t;
            self.thumbs.insert(id, t);
        }
        changed
    }
}

/// Checks the open project's media files and loads missing thumbnails on a background thread, then broadcasts the
/// view if anything shown changed. Runs at most one check at a time (a request during a check runs it again after).
/// No lock is held while the disk is accessed, so an offline network volume only delays this thread.
pub fn refresh_media(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let r = &state.projects.refresh;
    r.again.store(true, Ordering::SeqCst);
    if r.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = worker::spawn_contained("media check", move || {
        let state = app.state::<AppState>();
        let r = &state.projects.refresh;
        loop {
            while r.again.swap(false, Ordering::SeqCst) {
                let _ = worker::contain("the media check", || refresh_once(&app, &state));
            }
            r.running.store(false, Ordering::SeqCst);
            // A request that arrived after the last pass and before `running` cleared is ours to run.
            if !r.again.load(Ordering::SeqCst) || r.running.swap(true, Ordering::SeqCst) {
                break;
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the media check");
        state.projects.refresh.running.store(false, Ordering::SeqCst);
    }
}

fn refresh_once(app: &AppHandle, state: &AppState) {
    let snapshot =
        lock(&state.projects.open).ok().and_then(|g| g.as_ref().map(|p| (p.bundle().to_path_buf(), p.doc().clone())));
    let Some((bundle, doc)) = snapshot else { return };
    let have_thumb: BTreeSet<String> = {
        let mut c = relock(&state.projects.media);
        c.select(&bundle);
        c.thumbs.iter().filter(|(_, t)| t.is_some()).map(|(id, _)| id.clone()).collect()
    };
    let links = media_status(&bundle, &doc)
        .into_iter()
        .filter_map(|s| {
            let key = link_key(doc.registry.get(&s.media_id)?);
            Some((s.media_id, key, LinkView { path: s.path.display().to_string(), state: s.state }))
        })
        .collect();
    let thumbs = doc
        .project
        .media
        .keys()
        .filter(|id| !have_thumb.contains(*id))
        .map(|id| (id.clone(), thumbnail_data_url(&bundle, id)))
        .collect();
    // Released before the project is locked: the lock order is project, then media cache.
    let changed = relock(&state.projects.media).store(&bundle, links, thumbs);
    if changed
        && let Ok(g) = lock(&state.projects.open)
        && let Some(p) = g.as_ref().filter(|p| p.bundle() == bundle)
    {
        let _ = ProjectChanged(Some(cached_view(app, state, p))).emit(app);
    }
}

/// The view of the open project from the media cache; schedules a media check when the cache is behind.
pub(crate) fn cached_view(app: &AppHandle, state: &AppState, p: &Project) -> ProjectView {
    let (v, stale) = relock(&state.projects.media).view(p);
    if stale {
        refresh_media(app);
    }
    v
}

/// Default tracks of a new project, top layer first.
pub(crate) fn default_tracks() -> Vec<Op> {
    ove_store::library::starter_tracks()
}

/// Broadcasts the open project's view and returns it.
pub(crate) fn emit_view(app: &AppHandle, p: Option<&Project>) -> Option<ProjectView> {
    let state = app.state::<AppState>();
    let v = p.map(|p| cached_view(app, &state, p));
    let _ = ProjectChanged(v.clone()).emit(app);
    // The timeline preview follows every change.
    if let Some(p) = p {
        crate::exporting::notify_doc(&state, p.doc(), p.bundle());
    }
    v
}

pub(crate) fn emit(app: &AppHandle, p: Option<&Project>) {
    emit_view(app, p);
}

/// Runs a project command off the main thread, after every project command that arrived before it (one at a time,
/// in order), with panics contained. `f` gets the app and its state.
pub(crate) async fn serial<T: Send + 'static>(
    app: &AppHandle,
    name: &'static str,
    f: impl FnOnce(&AppHandle, &AppState) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let state = app.state::<AppState>();
    let _turn = state.projects.turn.lock().await;
    let a = app.clone();
    worker::blocking(name, move || f(&a, &a.state::<AppState>())).await
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
pub async fn list_projects(app: AppHandle) -> AppResult<Vec<ProjectSummary>> {
    worker::blocking("list_projects", move || {
        let state = app.state::<AppState>();
        let lib = library(&state.projects)?;
        summaries(lib.as_ref().unwrap())
    })
    .await
}

fn close_open(state: &AppState) -> AppResult<()> {
    if let Some(p) = lock(&state.projects.open)?.take() {
        p.close()?;
    }
    Ok(())
}

/// Makes `p` the open project and broadcasts it.
fn install(app: &AppHandle, state: &AppState, p: Project) -> AppResult<ProjectView> {
    let mut g = lock(&state.projects.open)?;
    *g = Some(p);
    emit_view(app, g.as_ref()).ok_or_else(|| AppError::new(ErrorCode::Internal, "The project did not open."))
}

#[tauri::command]
#[specta::specta]
pub async fn create_project(app: AppHandle, name: String, template: Option<String>) -> AppResult<ProjectView> {
    serial(&app, "create_project", move |app, state| {
        close_open(state)?;
        let mut p = {
            let lib = library(&state.projects)?;
            let name = if name.trim().is_empty() { "Untitled Project".to_string() } else { name };
            lib.as_ref().unwrap().create(&name)?
        };
        // The history panel attributes a project started from a template to it.
        let summary = match template.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => format!("{TEMPLATE_SUMMARY}“{t}”"),
            None => "new project".into(),
        };
        p.apply(default_tracks(), Source::Gui, Some(summary))?;
        tracing::info!(bundle = %p.bundle().display(), "project created");
        install(app, state, p)
    })
    .await
}

/// Opens a project bundle for editing (closing the one open). Used by the command and the control link.
pub(crate) fn open_bundle(app: &AppHandle, state: &AppState, bundle: &str) -> AppResult<ProjectView> {
    close_open(state)?;
    let p = Project::open(Path::new(bundle), Mode::Write, clock())?;
    if let Some(prev) = &p.took_over_from {
        tracing::warn!(previous = %prev.describe(), "recovered a project left open by a process that stopped");
    }
    install(app, state, p)
}

#[tauri::command]
#[specta::specta]
pub async fn open_project(app: AppHandle, bundle: String) -> AppResult<ProjectView> {
    serial(&app, "open_project", move |app, state| open_bundle(app, state, &bundle)).await
}

#[tauri::command]
#[specta::specta]
pub async fn close_project(app: AppHandle) -> AppResult<()> {
    serial(&app, "close_project", |app, state| {
        close_open(state)?;
        let _ = state.engine.send(crate::engine::EngineMsg::Close);
        emit(app, None);
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn project_view(app: AppHandle) -> AppResult<Option<ProjectView>> {
    serial(&app, "project_view", |app, state| {
        Ok(lock(&state.projects.open)?.as_ref().map(|p| cached_view(app, state, p)))
    })
    .await
}

/// Applies ops to the open project as one revision (nothing to apply: no revision).
fn commit<'a>(open: &'a mut Option<Project>, ops: Vec<Op>, summary: &str) -> AppResult<&'a mut Project> {
    commit_as(open, ops, summary, Source::Gui)
}

/// [`commit`] attributed to `source`.
fn commit_as<'a>(
    open: &'a mut Option<Project>,
    ops: Vec<Op>,
    summary: &str,
    source: Source,
) -> AppResult<&'a mut Project> {
    let p = open.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
    if !ops.is_empty() {
        p.apply(ops, source, Some(summary.to_string()))?;
    }
    Ok(p)
}

/// Applies ops to the open project as one revision and broadcasts the new view.
pub(crate) fn edit(app: &AppHandle, state: &AppState, ops: Vec<Op>, summary: &str) -> AppResult<ProjectView> {
    let mut g = lock(&state.projects.open)?;
    let p = commit(&mut g, ops, summary)?;
    emit_view(app, Some(p)).ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
}

/// Makes a 160×90 PNG thumbnail from the frame at 1 s (or the first frame).
pub(crate) fn make_thumbnail(path: &Path, out: &Path, duration_us: i64) -> Option<()> {
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

/// The media record and registry entry of a probed file, and its duration in µs.
pub(crate) fn media_records(canonical: &Path, info: &ove_media::MediaInfo, id: &str, now: i64) -> (Value, Value, i64) {
    let name = canonical.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let duration_us = info.duration_us.unwrap_or(5_000_000).max(40_000);
    let mut data = Map::new();
    data.insert("type".into(), (if info.video.is_some() { "video" } else { "audio" }).into());
    data.insert("fileName".into(), name.into());
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
    let mut entry = json!({"mediaId": id, "nativePath": canonical.display().to_string(), "ownedByBundle": false, "probeStatus": "probed"});
    if let Ok(md) = std::fs::metadata(canonical) {
        entry["size"] = json!(md.len());
        if let Some(t) = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
            entry["modifiedAt"] = json!(t.as_millis() as i64);
        }
    }
    (json!({"id": id, "createdAt": now, "addedAt": now, "data": Value::Object(data)}), entry, duration_us)
}

/// One file ready to add: its records and where it goes.
struct Probed {
    media: Value,
    entry: Value,
    id: String,
    name: String,
    duration_us: i64,
    video: bool,
}

/// Probes files, registers their records and makes thumbnails. Slow (decoding), so it runs outside the project's
/// turn: edits made meanwhile are not held up.
fn probe_paths(state: &AppState, paths: &[String], now: i64) -> AppResult<(PathBuf, Vec<Probed>, Vec<String>)> {
    let bundle = {
        let g = lock(&state.projects.open)?;
        let p =
            g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open or create a project first."))?;
        p.bundle().to_path_buf()
    };
    let mut probed = Vec::new();
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
        let (media, entry, duration_us) = media_records(&canonical, &info, &id, now);
        if info.video.is_some() {
            let out = thumbs_dir(&bundle).join(format!("{id}.png"));
            if make_thumbnail(&canonical, &out, duration_us).is_none() {
                tracing::warn!(path, "no thumbnail");
            }
            // Read once here; views reuse it.
            relock(&state.projects.media).put_thumbnail(&bundle, &id, thumbnail_data_url(&bundle, &id));
        } else {
            relock(&state.projects.media).put_thumbnail(&bundle, &id, None);
        }
        probed.push(Probed { media, entry, id, name, duration_us, video: info.video.is_some() });
    }
    Ok((bundle, probed, errors))
}

/// Appends probed files to the Main track (audio-only files to the Audio track) as one revision.
fn add_probed(
    app: &AppHandle,
    state: &AppState,
    bundle: &Path,
    probed: Vec<Probed>,
    now: i64,
    source: Source,
) -> AppResult<ProjectView> {
    let mut g = lock(&state.projects.open)?;
    let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open or create a project first."))?;
    if p.bundle() != bundle {
        return Err(AppError::new(ErrorCode::InvalidArgument, "The project changed while the files were read."));
    }
    let doc = p.doc();
    let known: BTreeSet<&str> = doc.registry.values().map(|e| e.native_path.as_str()).collect();
    let track_end = |track: &str| doc.items_on_track(track).iter().map(|i| i.end).max().unwrap_or(0);
    let (mut main_end, mut audio_end) = (track_end("t_main"), track_end("t_audio"));
    let mut ops = Vec::new();
    for (n, f) in probed.into_iter().enumerate() {
        if f.entry.get("nativePath").and_then(Value::as_str).is_some_and(|p| known.contains(p)) {
            tracing::info!(name = f.name, "media already in the project; adding another clip");
        }
        ops.push(Op::AddMedia { media: f.media, entry: Some(f.entry) });
        let (track, kind, at) = if f.video {
            main_end += f.duration_us;
            ("t_main", "video", main_end - f.duration_us)
        } else {
            audio_end += f.duration_us;
            ("t_audio", "audio", audio_end - f.duration_us)
        };
        ops.push(Op::AddItem {
            item: json!({"id": format!("i_{:x}_{n}", now), "trackId": track, "type": kind,
                "startTime": ove_core::time::micros_to_json(at), "endTime": ove_core::time::micros_to_json(at + f.duration_us),
                "name": f.name, "mediaItemId": f.id}),
        });
    }
    let count = ops.len() / 2;
    let summary = format!(
        "{}import {count} file{}",
        if source == Source::Agent { "assistant: " } else { "" },
        if count == 1 { "" } else { "s" }
    );
    let p = commit_as(&mut g, ops, &summary, source)?;
    emit_view(app, Some(p)).ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
}

#[tauri::command]
#[specta::specta]
pub async fn import_media(app: AppHandle, paths: Vec<String>) -> AppResult<ProjectView> {
    import_paths(app, paths, Source::Gui).await
}

/// Imports files as one revision attributed to `source` (the editor, or the assistant's `import_media` tool).
pub(crate) async fn import_paths(app: AppHandle, paths: Vec<String>, source: Source) -> AppResult<ProjectView> {
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
    let a = app.clone();
    let (bundle, probed, errors) =
        worker::blocking("import_media", move || probe_paths(&a.state::<AppState>(), &paths, now)).await?;
    let v =
        serial(&app, "import_media", move |app, state| add_probed(app, state, &bundle, probed, now, source)).await?;
    if !errors.is_empty() {
        let _ = crate::AppErrorRaised(AppError::new(ErrorCode::Unsupported, errors.join(" "))).emit(&app);
    }
    Ok(v)
}

#[tauri::command]
#[specta::specta]
pub async fn import_media_dialog(app: AppHandle) -> AppResult<Option<ProjectView>> {
    let mut exts: Vec<&str> = crate::media::VIDEO_EXTENSIONS.to_vec();
    exts.extend_from_slice(crate::media::AUDIO_EXTENSIONS);
    let a = app.clone();
    let files = worker::blocking("import_media_dialog", move || {
        Ok(a.dialog().file().add_filter("Media", &exts).blocking_pick_files())
    })
    .await?;
    let Some(files) = files else { return Ok(None) };
    let paths: Vec<String> =
        files.into_iter().filter_map(|f| f.into_path().ok()).map(|p| p.display().to_string()).collect();
    import_media(app, paths).await.map(Some)
}

/// Moves an item to a new start time (and optionally another track), keeping its duration.
#[tauri::command]
#[specta::specta]
pub async fn move_item(app: AppHandle, id: String, start: f64, track_id: Option<String>) -> AppResult<ProjectView> {
    serial(&app, "move_item", move |app, state| {
        let (old_start, old_end) = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
            let i =
                p.doc().item(&id).ok_or_else(|| AppError::new(ErrorCode::NotFound, "That clip no longer exists."))?;
            (i.start, i.end)
        };
        let new_start = ove_core::time::secs_to_micros(start.max(0.0)).unwrap_or(old_start);
        let mut set = Map::new();
        set.insert("startTime".into(), ove_core::time::micros_to_json(new_start));
        set.insert("endTime".into(), ove_core::time::micros_to_json(new_start + (old_end - old_start)));
        if let Some(t) = track_id {
            set.insert("trackId".into(), t.into());
        }
        edit(app, state, vec![Op::UpdateItem { id, set }], "move clip")
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_items(app: AppHandle, ids: Vec<String>) -> AppResult<ProjectView> {
    serial(&app, "remove_items", move |app, state| {
        let n = ids.len();
        edit(app, state, vec![Op::RemoveItems { ids }], &format!("delete {n} item{}", if n == 1 { "" } else { "s" }))
    })
    .await
}

/// Splits a clip at a timeline time (seconds): the left part keeps the id, the right part is a new item.
#[tauri::command]
#[specta::specta]
pub async fn split_item(app: AppHandle, id: String, at: f64) -> AppResult<ProjectView> {
    serial(&app, "split_item", move |app, state| {
        let item = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
            p.doc()
                .item(&id)
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "That clip no longer exists."))?
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
        edit(app, state, vec![Op::UpdateItem { id, set }, Op::AddItem { item: right }], "split clip")
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_aspect_ratio(app: AppHandle, aspect: String) -> AppResult<ProjectView> {
    serial(&app, "set_aspect_ratio", move |app, state| {
        edit(app, state, vec![Op::SetAspectRatio { aspect_ratio: aspect }], "change aspect ratio")
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_project(app: AppHandle, name: String) -> AppResult<ProjectView> {
    serial(&app, "rename_project", move |app, state| {
        let mut set = Map::new();
        set.insert("name".into(), name.into());
        edit(app, state, vec![Op::UpdateProject { set, unset: vec![] }], "rename project")
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_track_flag(app: AppHandle, id: String, flag: String, value: bool) -> AppResult<ProjectView> {
    if !matches!(flag.as_str(), "muted" | "visible" | "locked") {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Unknown track setting."));
    }
    serial(&app, "set_track_flag", move |app, state| {
        let mut set = Map::new();
        set.insert(flag, value.into());
        edit(app, state, vec![Op::UpdateTrack { id, set }], "track setting")
    })
    .await
}

/// How a project started from a template is summarised in its first revision.
pub(crate) const TEMPLATE_SUMMARY: &str = "new project from template ";

/// One revision in the history panel.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub rev: f64,
    /// ms since the epoch.
    pub time: f64,
    /// The recorded source: gui, cli, mcp, agent, migration, undo, redo or restore.
    pub source: String,
    /// Who made it, in words: You, Assistant, Command line, MCP, Template, Generation, App update.
    pub who: String,
    pub summary: String,
    /// The project as it was before this revision can still be rebuilt (older ones may have been pruned).
    pub can_restore: bool,
}

/// Who a revision is attributed to, as far as its record tells.
pub fn who(source: Source, summary: &str) -> &'static str {
    match source {
        Source::Agent => "Assistant",
        Source::Cli => "Command line",
        Source::Mcp => "MCP",
        Source::Migration => "App update",
        Source::Gui if summary.starts_with(TEMPLATE_SUMMARY) => "Template",
        Source::Gui if summary.starts_with("generate") => "Generation",
        Source::Gui | Source::Undo | Source::Redo | Source::Restore => "You",
    }
}

/// The newest `limit` revisions of a project, newest first.
pub fn history_rows(p: &Project, limit: usize) -> AppResult<Vec<HistoryRow>> {
    let rows: Vec<HistoryRow> = p
        .history()?
        .into_iter()
        .rev()
        .take(limit)
        .map(|h| HistoryRow {
            rev: h.rev as f64,
            time: h.time as f64,
            source: h.source.as_str().into(),
            who: who(h.source, &h.summary).into(),
            can_restore: p.is_retained(h.parent),
            summary: h.summary,
        })
        .collect();
    Ok(rows)
}

/// Restores the project to how it was before revision `rev`, as one new (undoable) revision.
pub fn restore_before_rev(p: &mut Project, rev: i64) -> AppResult<()> {
    let rec = ove_store::project::read_record(p.bundle(), rev)?;
    if !p.is_retained(rec.parent) {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "That change is too old to go back to: older history is cleared after 30 days or 2,000 changes.",
        ));
    }
    let short: String = rec.summary.chars().take(60).collect();
    let summary = if short.is_empty() {
        format!("restore to before revision {rev}")
    } else {
        format!("restore to before “{short}”")
    };
    p.restore_with(rec.parent, Source::Restore, Some(summary))?;
    Ok(())
}

/// The open project's revisions, newest first (the history panel).
#[tauri::command]
#[specta::specta]
pub async fn project_history(app: AppHandle) -> AppResult<Vec<HistoryRow>> {
    serial(&app, "project_history", |_, state| {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        history_rows(p, 500)
    })
    .await
}

/// "Restore to before this": one new revision with the project as it was before `rev`.
#[tauri::command]
#[specta::specta]
pub async fn restore_before(app: AppHandle, rev: f64) -> AppResult<ProjectView> {
    serial(&app, "restore_before", move |app, state| {
        let mut g = lock(&state.projects.open)?;
        let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        restore_before_rev(p, rev as i64)?;
        emit_view(app, Some(p)).ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn undo(app: AppHandle) -> AppResult<ProjectView> {
    serial(&app, "undo", |app, state| {
        let mut g = lock(&state.projects.open)?;
        let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        p.undo()?;
        emit_view(app, Some(p)).ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn redo(app: AppHandle) -> AppResult<ProjectView> {
    serial(&app, "redo", |app, state| {
        let mut g = lock(&state.projects.open)?;
        let p = g.as_mut().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
        p.redo()?;
        emit_view(app, Some(p)).ok_or_else(|| AppError::new(ErrorCode::Internal, "No project is open."))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_project(lib: &Library) -> Projects {
        let mut p = lib.create("Trip").unwrap();
        p.apply(default_tracks(), Source::Gui, None).unwrap();
        let projects = Projects::default();
        *relock(&projects.open) = Some(p);
        projects
    }

    fn rename(name: &str) -> Vec<Op> {
        let mut set = Map::new();
        set.insert("name".into(), name.into());
        vec![Op::UpdateProject { set, unset: vec![] }]
    }

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

    #[test]
    fn history_lists_who_did_what_newest_first_and_restores_before_a_change() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let mut p = lib.create("Trip").unwrap();
        p.apply(default_tracks(), Source::Gui, Some(format!("{TEMPLATE_SUMMARY}“Event recap”"))).unwrap();
        p.apply(rename("By hand"), Source::Gui, Some("rename project".into())).unwrap();
        p.apply(rename("By agent"), Source::Agent, Some("assistant: updated 1".into())).unwrap();
        p.apply(rename("By cli"), Source::Cli, None).unwrap();
        p.apply(rename("By mcp"), Source::Mcp, None).unwrap();
        p.apply(rename("Generated"), Source::Gui, Some("generate image: placeholder".into())).unwrap();
        let rows = history_rows(&p, 100).unwrap();
        let who: Vec<&str> = rows.iter().map(|r| r.who.as_str()).collect();
        assert_eq!(who, ["Generation", "MCP", "Command line", "Assistant", "You", "Template"]);
        assert!(rows.windows(2).all(|w| w[0].rev > w[1].rev));
        assert!(rows.iter().all(|r| r.can_restore));
        assert_eq!(history_rows(&p, 2).unwrap().len(), 2);

        // Restore to before the assistant's change: one new, undoable revision.
        let agent_rev = rows[3].rev as i64;
        restore_before_rev(&mut p, agent_rev).unwrap();
        assert_eq!(p.doc().project.name, "By hand");
        let latest = p.latest_record().unwrap().unwrap();
        assert_eq!(latest.source, Source::Restore);
        assert_eq!(latest.summary, "restore to before “assistant: updated 1”");
        assert_eq!(history_rows(&p, 1).unwrap()[0].who, "You");
        p.undo().unwrap();
        assert_eq!(p.doc().project.name, "Generated");
        assert!(restore_before_rev(&mut p, 12345).is_err());
    }

    #[test]
    fn edit_after_a_panicking_lock_holder_succeeds() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let projects = Arc::new(open_project(&lib));
        let rev = relock(&projects.open).as_ref().unwrap().rev();

        // A command panics while holding the open project (as a bug in an edit would).
        let p2 = projects.clone();
        let err = worker::command("move_item", move || -> AppResult<()> {
            let _g = lock(&p2.open)?;
            panic!("injected failure");
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkerPanic);
        assert!(projects.open.is_poisoned());

        // The next edit works, and the lock is clean again.
        let mut g = lock(&projects.open).unwrap();
        let p = commit(&mut g, rename("After"), "rename").unwrap();
        assert!(p.rev() > rev);
        let rev = p.rev();
        assert_eq!(p.doc().project.name, "After");
        drop(g);
        assert!(!projects.open.is_poisoned());
        let mut g = lock(&projects.open).unwrap();
        assert!(commit(&mut g, rename("Again"), "rename").unwrap().rev() > rev);
    }

    #[test]
    fn media_cache_view_does_not_touch_the_disk_until_refreshed() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let projects = open_project(&lib);
        let mut g = lock(&projects.open).unwrap();
        let p = g.as_mut().unwrap();
        let missing = d.path().join("gone.mp4");
        let (media, entry, _) = media_records(
            &missing,
            &ove_media::MediaInfo {
                path: missing.clone(),
                container: "mp4".into(),
                duration_us: Some(1_000_000),
                bit_rate: None,
                video: None,
                audio: None,
                stream_count: 1,
            },
            "m1",
            0,
        );
        p.apply(vec![Op::AddMedia { media, entry: Some(entry) }], Source::Gui, None).unwrap();

        let mut cache = MediaCache::default();
        // Unchecked: shown online, flagged for a check.
        let (v, stale) = cache.view(p);
        assert!(stale);
        assert_eq!(v.media[0].status, "online");
        // The background check finds it offline.
        let links = media_status(p.bundle(), p.doc())
            .into_iter()
            .map(|s| {
                let key = link_key(&p.doc().registry[&s.media_id]);
                (s.media_id, key, LinkView { path: s.path.display().to_string(), state: s.state })
            })
            .collect();
        assert!(cache.store(p.bundle(), links, vec![("m1".into(), None)]));
        let (v, stale) = cache.view(p);
        assert!(!stale);
        assert_eq!(v.media[0].status, "offline");
        // The full (uncached) view agrees.
        assert_eq!(view(p).media[0].status, "offline");
    }
}
