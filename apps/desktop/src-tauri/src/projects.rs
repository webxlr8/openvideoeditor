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
    /// Ripple edits on other tracks shift this track too.
    pub sync_lock: bool,
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
    /// Link group value, when the item is linked.
    pub link: Option<String>,
    /// Volume, fades and volume keyframes of video and audio items, for the waveform and gain line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub audio: Option<crate::waveforms::ItemAudio>,
    /// Keyframe tracks other than volume (opacity, scale, crop, …), for the keyframe lanes (absent: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub keyframes: Option<Vec<crate::keyframes::KeyframeTrackView>>,
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
    /// The hover-skim sheet, once built (served at `ove-skim://localhost/<contentId>.jpg`).
    pub skim: Option<SkimView>,
}

/// A skim sheet's image and layout: tile `i` of `count` sits at column `i % columns`, row `i / columns`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkimView {
    pub content_id: String,
    pub columns: u32,
    pub rows: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub count: u32,
}

impl SkimView {
    fn new(content_id: &str, s: &ove_media::skim::SkimSheet) -> SkimView {
        SkimView {
            content_id: content_id.to_string(),
            columns: s.columns,
            rows: s.rows,
            tile_width: s.tile_width,
            tile_height: s.tile_height,
            count: s.times_us.len() as u32,
        }
    }

    /// The sheet of `content_id` in `bundle`, read from disk.
    pub(crate) fn read(bundle: &Path, content_id: &str) -> Option<SkimView> {
        crate::tasks::read_skim(bundle, content_id).map(|s| SkimView::new(content_id, &s))
    }
}

/// The content id of a media item, when its registry entry has one.
fn content_id_of<'a>(doc: &'a ove_core::Document, media_id: &str) -> Option<&'a str> {
    doc.registry.get(media_id)?.fields.get("contentId")?.as_str()
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
    /// Timeline frame rate (edits snap to its frame grid).
    pub fps: f64,
    pub duration_seconds: f64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_summary: Option<String>,
    pub tracks: Vec<TrackView>,
    pub items: Vec<ItemView>,
    pub media: Vec<MediaView>,
    /// The project's "Use proxies" setting (absent: off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub use_proxies: Option<bool>,
    /// Timeline markers in time order (absent: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub markers: Option<Vec<crate::markers::MarkerView>>,
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

/// Runs `f` with the library (opened on first use), holding its lock.
pub(crate) fn with_library<T>(p: &Projects, f: impl FnOnce(&Library) -> AppResult<T>) -> AppResult<T> {
    let mut g = lock(&p.library)?;
    let lib = match g.take() {
        Some(lib) => lib,
        None => Library::open(ove_store::library::resolve_library(None)?, clock())?,
    };
    f(g.insert(lib))
}

pub(crate) fn thumbs_dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("thumbs")
}

/// `cache/thumbs/<key>.png` as a data URL.
fn thumbnail_data_url(bundle: &Path, key: &str) -> Option<String> {
    let bytes = std::fs::read(thumbs_dir(bundle).join(format!("{key}.png"))).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// What a media file's thumbnail is stored under: its content id (so a moved or relinked file keeps it), or its
/// media id for entries from before content ids.
pub(crate) fn thumb_key(doc: &ove_core::Document, media_id: &str) -> String {
    doc.registry
        .get(media_id)
        .and_then(|e| e.fields.get("contentId"))
        .and_then(Value::as_str)
        .map_or_else(|| media_id.to_string(), str::to_string)
}

/// A media file's thumbnail: under its content id, else under its media id (not migrated yet, or written by a
/// feature that names thumbnails by media id).
fn thumbnail_for(bundle: &Path, doc: &ove_core::Document, media_id: &str) -> Option<String> {
    thumbnail_data_url(bundle, &thumb_key(doc, media_id)).or_else(|| thumbnail_data_url(bundle, media_id))
}

/// Moves derived data from before content ids onto them: thumbnails named by media id and transcripts keyed by
/// path, size and time. Returns how many files moved. Entries without a content id are left alone.
#[cfg(test)]
pub(crate) fn migrate_derived(bundle: &Path, doc: &ove_core::Document) -> usize {
    let all: BTreeSet<String> = doc
        .registry
        .values()
        .filter_map(|e| e.fields.get("contentId").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    migrate_derived_only(bundle, doc, &all)
}

/// [`migrate_derived`] for the entries whose content id is in `only`.
fn migrate_derived_only(bundle: &Path, doc: &ove_core::Document, only: &BTreeSet<String>) -> usize {
    let mut moved = 0;
    for e in doc.registry.values() {
        let Some(cid) = e.fields.get("contentId").and_then(Value::as_str).filter(|c| only.contains(*c)) else {
            continue;
        };
        let old = thumbs_dir(bundle).join(format!("{}.png", e.media_id));
        if old.is_file() {
            let new = thumbs_dir(bundle).join(format!("{cid}.png"));
            let ok = if new.exists() { std::fs::remove_file(&old) } else { std::fs::rename(&old, &new) };
            moved += usize::from(ok.is_ok());
        }
        let path = resolve_media_path(bundle, &e.native_path);
        moved += ove_transcribe::cache::migrate_legacy(bundle, &path, cid);
    }
    moved
}

/// Computes content ids in a background task for entries without one (projects from before content ids) or whose
/// file changed, and stores them as one revision attributed to migration. The next media check then moves older
/// derived data onto them.
fn queue_identity_check(app: &AppHandle, bundle: &Path) {
    let state = app.state::<AppState>();
    let spec =
        ove_tasks::TaskSpec::new("identity", "Media identities", ove_tasks::Priority::Derivative, ove_tasks::Class::Io)
            .project(crate::tasks::project_key(bundle))
            .memory(8 * 1024 * 1024)
            .key(format!("identity:{}", bundle.display()));
    let (a, b) = (app.clone(), bundle.to_path_buf());
    state.tasks.submit(spec, move |ctx| {
        let state = a.state::<AppState>();
        let doc = {
            let g = lock(&state.projects.open).map_err(|e| ove_tasks::TaskError::Failed(e.message))?;
            match g.as_ref() {
                Some(p) if p.bundle() == b => p.doc().clone(),
                _ => return Ok(()),
            }
        };
        ctx.checkpoint()?;
        let ops = identity_ops(&b, &doc);
        ctx.checkpoint()?;
        if ops.is_empty() {
            return Ok(());
        }
        let n = ops.len();
        {
            let mut g = lock(&state.projects.open).map_err(|e| ove_tasks::TaskError::Failed(e.message))?;
            // Only onto the same project, unchanged since the files were read.
            if !g.as_ref().is_some_and(|p| p.bundle() == b && p.doc().registry == doc.registry) {
                return Ok(());
            }
            let p = commit_as(&mut g, ops, "media identities", Source::Migration)
                .map_err(|e| ove_tasks::TaskError::Failed(e.message))?;
            emit_view(&a, Some(p));
        }
        tracing::info!(entries = n, "stored media content ids");
        refresh_media(&a);
        Ok(())
    });
}

/// Ops that store content ids for entries that lack one (projects from before content ids) or whose file changed
/// in place. Reads the files; runs as a background task.
pub(crate) fn identity_ops(bundle: &Path, doc: &ove_core::Document) -> Vec<Op> {
    doc.registry
        .values()
        .filter_map(|e| {
            let path = resolve_media_path(bundle, &e.native_path);
            let entry_set = ove_store::media::refresh_identity(&path, &e.fields)?;
            Some(Op::UpdateMedia {
                id: e.media_id.clone(),
                set: Map::new(),
                unset: vec![],
                entry_set,
                entry_unset: vec![],
            })
        })
        .collect()
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
    let v = view_from(
        p,
        |id| {
            status
                .iter()
                .find(|s| s.media_id == id)
                .map(|s| LinkView { path: s.path.display().to_string(), state: s.state })
        },
        |id| thumbnail_for(p.bundle(), p.doc(), id),
    );
    let mut v = v;
    for m in &mut v.media {
        m.skim = content_id_of(p.doc(), &m.id).and_then(|cid| SkimView::read(p.bundle(), cid));
    }
    v
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
                skim: None,
            }
        })
        .collect();
    ProjectView {
        id: d.project.id.clone(),
        name: d.project.name.clone(),
        bundle: p.bundle().display().to_string(),
        rev: d.project.rev as f64,
        aspect_ratio: d.project.aspect_ratio.clone(),
        fps: d.fps(),
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
                sync_lock: track_bool(t, "syncLock", true),
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
                link: i.fields.get("link").and_then(Value::as_str).map(str::to_string),
                audio: crate::waveforms::item_audio(i),
                keyframes: crate::keyframes::views(i),
            })
            .collect(),
        media,
        use_proxies: ove_render::proxy::enabled(d).then_some(true),
        markers: crate::markers::views(d),
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
    /// Media whose missing thumbnail was queued once already (a file that cannot be thumbnailed is not retried).
    thumbs_queued: BTreeSet<String>,
    /// Content ids whose older derived data was moved onto them this session.
    migrated: BTreeSet<String>,
    /// Registry states (link keys) an identity check was queued for.
    identity_queued: BTreeSet<String>,
    /// Content ids a skim sheet was queued for (or found finished).
    skim_queued: BTreeSet<String>,
    /// content id → finished skim sheet.
    skims: HashMap<String, SkimView>,
    /// Content ids an audio analysis was queued for (or found finished).
    analysis_queued: BTreeSet<String>,
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
        let mut v = v;
        for m in &mut v.media {
            m.skim = content_id_of(d, &m.id).and_then(|cid| self.skims.get(cid).cloned());
        }
        (v, stale)
    }

    /// Forgets everything checked and derived, so the next media check reads it all again from disk (after
    /// Settings → Storage cleared a cache).
    pub fn forget(&mut self) {
        *self = MediaCache::default();
    }

    /// Records a finished skim sheet.
    pub fn put_skim(&mut self, bundle: &Path, skim: SkimView) {
        self.select(bundle);
        self.skims.insert(skim.content_id.clone(), skim);
    }

    /// The bundle whose media this cache holds (the open project's).
    pub fn bundle(&self) -> &Path {
        &self.bundle
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
            // A thumbnail task may have finished after this check looked for the file.
            if t.is_none() && self.thumbs.get(&id).is_some_and(Option::is_some) {
                continue;
            }
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
    let (have_thumb, to_migrate, needs_identity) = {
        let mut c = relock(&state.projects.media);
        c.select(&bundle);
        let have: BTreeSet<String> = c.thumbs.iter().filter(|(_, t)| t.is_some()).map(|(id, _)| id.clone()).collect();
        let to_migrate: BTreeSet<String> = doc
            .registry
            .values()
            .filter_map(|e| e.fields.get("contentId").and_then(Value::as_str))
            .filter(|cid| !c.migrated.contains(*cid))
            .map(str::to_string)
            .collect();
        c.migrated.extend(to_migrate.iter().cloned());
        let keys: Vec<String> = doc.registry.values().map(link_key).collect();
        let needs_identity = keys.iter().any(|k| !c.identity_queued.contains(k));
        c.identity_queued.extend(keys);
        (have, to_migrate, needs_identity)
    };
    // Derived data from before content ids moves onto them (first open after the upgrade, or after an identity check).
    if !to_migrate.is_empty() {
        let moved = migrate_derived_only(&bundle, &doc, &to_migrate);
        if moved > 0 {
            tracing::info!(moved, "moved derived data onto content ids");
        }
    }
    if needs_identity {
        queue_identity_check(app, &bundle);
    }
    let links: Vec<(String, String, LinkView)> = media_status(&bundle, &doc)
        .into_iter()
        .filter_map(|s| {
            let key = link_key(doc.registry.get(&s.media_id)?);
            Some((s.media_id, key, LinkView { path: s.path.display().to_string(), state: s.state }))
        })
        .collect();
    let thumbs: Vec<(String, Option<String>)> = doc
        .project
        .media
        .keys()
        .filter(|id| !have_thumb.contains(*id))
        .map(|id| (id.clone(), thumbnail_for(&bundle, &doc, id)))
        .collect();
    // Thumbnails are derived data: one that is missing (a cleared cache, an older project) is made again.
    let missing: Vec<(String, String, PathBuf, i64)> = {
        let mut c = relock(&state.projects.media);
        let online: BTreeSet<&str> =
            links.iter().filter(|(_, _, l)| l.state == MediaState::Online).map(|(id, _, _)| id.as_str()).collect();
        thumbs
            .iter()
            .filter(|(id, t)| t.is_none() && online.contains(id.as_str()))
            .filter_map(|(id, _)| {
                let data = doc.project.media.get(id)?.fields.get("data")?;
                (data.get("type").and_then(Value::as_str) == Some("video")).then_some(())?;
                let e = doc.registry.get(id)?;
                let dur = data.get("duration").and_then(ove_core::time::json_to_micros).unwrap_or(1_000_000);
                c.thumbs_queued
                    .insert(id.clone())
                    .then(|| (id.clone(), thumb_key(&doc, id), resolve_media_path(&bundle, &e.native_path), dur))
            })
            .collect()
    };
    // Skim sheets, keyed by content id: queued once per content for online videos without a finished sheet.
    let skims: Vec<(String, PathBuf, i64)> = {
        let mut c = relock(&state.projects.media);
        let online: BTreeSet<&str> =
            links.iter().filter(|(_, _, l)| l.state == MediaState::Online).map(|(id, _, _)| id.as_str()).collect();
        doc.registry
            .values()
            .filter(|e| online.contains(e.media_id.as_str()))
            .filter_map(|e| {
                let cid = e.fields.get("contentId")?.as_str()?;
                let data = doc.project.media.get(&e.media_id)?.fields.get("data")?;
                (data.get("type").and_then(Value::as_str) == Some("video")).then_some(())?;
                let dur = data.get("duration").and_then(ove_core::time::json_to_micros).unwrap_or(1_000_000);
                c.skim_queued.insert(cid.to_string()).then_some(())?;
                if let Some(s) = SkimView::read(&bundle, cid) {
                    c.skims.insert(cid.to_string(), s);
                    return None;
                }
                let original = resolve_media_path(&bundle, &e.native_path);
                Some((cid.to_string(), crate::proxies::derivative_source(&bundle, &doc, cid, original), dur))
            })
            .collect()
    };
    // Proxies, when the project uses them: online videos without one.
    let online_videos: Vec<(String, PathBuf)> = {
        let online: BTreeSet<&str> =
            links.iter().filter(|(_, _, l)| l.state == MediaState::Online).map(|(id, _, _)| id.as_str()).collect();
        doc.registry
            .values()
            .filter(|e| online.contains(e.media_id.as_str()))
            .filter_map(|e| {
                let cid = e.fields.get("contentId")?.as_str()?;
                let data = doc.project.media.get(&e.media_id)?.fields.get("data")?;
                (data.get("type").and_then(Value::as_str) == Some("video")).then_some(())?;
                Some((cid.to_string(), resolve_media_path(&bundle, &e.native_path)))
            })
            .collect()
    };
    crate::proxies::queue_missing(app, &bundle, &doc, online_videos);
    // Audio analysis (peaks and loudness), keyed by content id: queued once per content for online media with sound.
    let analyses: Vec<(String, PathBuf)> = {
        let mut c = relock(&state.projects.media);
        let online: BTreeSet<&str> =
            links.iter().filter(|(_, _, l)| l.state == MediaState::Online).map(|(id, _, _)| id.as_str()).collect();
        doc.registry
            .values()
            .filter(|e| online.contains(e.media_id.as_str()))
            .filter_map(|e| {
                let cid = e.fields.get("contentId")?.as_str()?;
                let data = doc.project.media.get(&e.media_id)?.fields.get("data")?;
                crate::waveforms::media_has_sound(data).then_some(())?;
                c.analysis_queued.insert(cid.to_string()).then_some(())?;
                Some((cid.to_string(), resolve_media_path(&bundle, &e.native_path)))
            })
            .collect()
    };
    // Released before the project is locked: the lock order is project, then media cache.
    let changed = relock(&state.projects.media).store(&bundle, links, thumbs);
    if !missing.is_empty() {
        queue_thumbnails(app, &bundle, missing);
    }
    if !skims.is_empty() {
        let tasks = &state.tasks;
        let project = crate::tasks::project_key(&bundle);
        for (cid, source, dur) in skims {
            let (a, b, c) = (app.clone(), bundle.clone(), cid.clone());
            crate::tasks::queue_skim(tasks, &project, &bundle, &cid, source, dur, move |made| {
                if let Some(s) = made.then(|| SkimView::read(&b, &c)).flatten() {
                    relock(&a.state::<AppState>().projects.media).put_skim(&b, s);
                    emit_view_soon(&a);
                }
            });
        }
    }
    if !analyses.is_empty() {
        let project = crate::tasks::project_key(&bundle);
        for (cid, source) in analyses {
            let (a, c) = (app.clone(), cid.clone());
            crate::waveforms::queue_analysis(&state.tasks, &project, &bundle, &cid, source, move |ok| {
                crate::waveforms::emit_changed(&a, &c, ok);
            });
        }
    }
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
        with_library(&state.projects, summaries)
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
        let name = if name.trim().is_empty() { "Untitled Project".to_string() } else { name };
        let mut p = with_library(&state.projects, |lib| Ok(lib.create(&name)?))?;
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
    // size, modifiedAt and contentId: written with the add-media op, so they cost no extra revision.
    if let Some(e) = entry.as_object_mut() {
        e.extend(ove_store::media::identity_fields(canonical));
    }
    (json!({"id": id, "createdAt": now, "addedAt": now, "data": Value::Object(data)}), entry, duration_us)
}

/// One file ready to add: its records and where it goes.
struct Probed {
    media: Value,
    entry: Value,
    id: String,
    name: String,
    path: PathBuf,
    duration_us: i64,
    video: bool,
    /// The file has sound as well as picture.
    audio: bool,
}

/// Probes files (as interactive background tasks, several at once) and builds their records. Thumbnails come later,
/// as derivative tasks, once the files are in the project ([`queue_thumbnails`]). Runs outside the project's turn:
/// edits made meanwhile are not held up.
fn probe_paths(state: &AppState, paths: &[String], now: i64) -> AppResult<(PathBuf, Vec<Probed>, Vec<String>)> {
    let bundle = {
        let g = lock(&state.projects.open)?;
        let p =
            g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open or create a project first."))?;
        p.bundle().to_path_buf()
    };
    let project = crate::tasks::project_key(&bundle);
    let (probed, errors) = probed_records(crate::tasks::probe_all(&state.tasks, &project, paths), now);
    for f in probed.iter().filter(|f| !f.video) {
        relock(&state.projects.media).put_thumbnail(&bundle, &f.id, None);
    }
    Ok((bundle, probed, errors))
}

/// Records for every file that probed, and the error text of every one that did not.
fn probed_records(
    results: Vec<(PathBuf, Result<ove_media::MediaInfo, String>)>,
    now: i64,
) -> (Vec<Probed>, Vec<String>) {
    let mut probed = Vec::new();
    let mut errors = Vec::new();
    for (n, (canonical, result)) in results.into_iter().enumerate() {
        let info = match result {
            Ok(i) => i,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let name = canonical.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let id = format!("m_{:x}_{n}", now);
        let (media, entry, duration_us) = media_records(&canonical, &info, &id, now);
        let (video, audio) = (info.video.is_some(), info.audio.is_some());
        probed.push(Probed { media, entry, id, name, path: canonical, duration_us, video, audio });
    }
    (probed, errors)
}

/// Queues a thumbnail task for each video file; each finished thumbnail shows in the media panel (views go out at
/// most four times a second while thumbnails arrive).
/// `files`: media id, thumbnail key ([`thumb_key`]), source file, duration.
fn queue_thumbnails(app: &AppHandle, bundle: &Path, files: Vec<(String, String, PathBuf, i64)>) {
    let state = app.state::<AppState>();
    let project = crate::tasks::project_key(bundle);
    for (id, key, source, duration_us) in files {
        let out = thumbs_dir(bundle).join(format!("{key}.png"));
        let (a, b) = (app.clone(), bundle.to_path_buf());
        crate::tasks::queue_thumbnail(&state.tasks, &project, source, out, duration_us, move |made| {
            let state = a.state::<AppState>();
            let url = if made { thumbnail_data_url(&b, &key) } else { None };
            // Every media entry with the same content shares the thumbnail (and the task: same key, one run).
            let mut ids = vec![id];
            if let Ok(g) = lock(&state.projects.open)
                && let Some(p) = g.as_ref().filter(|p| p.bundle() == b)
            {
                ids.extend(p.doc().registry.keys().filter(|m| thumb_key(p.doc(), m) == key).cloned());
            }
            let mut cache = relock(&state.projects.media);
            for m in ids {
                cache.put_thumbnail(&b, &m, url.clone());
            }
            drop(cache);
            emit_view_soon(&a);
        });
    }
}

/// Sends the open project's view shortly, once for a burst of changes (thumbnails arriving).
pub(crate) fn emit_view_soon(app: &AppHandle) {
    static PENDING: AtomicBool = AtomicBool::new(false);
    if PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let a = app.clone();
    let spawned = std::thread::Builder::new().name("view-soon".into()).spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        PENDING.store(false, Ordering::SeqCst);
        let state = a.state::<AppState>();
        if let Ok(g) = lock(&state.projects.open)
            && let Some(p) = g.as_ref()
        {
            let _ = ProjectChanged(Some(cached_view(&a, &state, p))).emit(&a);
        }
    });
    if spawned.is_err() {
        PENDING.store(false, Ordering::SeqCst);
    }
}

/// The ops that add probed files: each file's media record, then its items appended to the Main track (audio-only
/// files to the Audio track). A file with picture and sound becomes a linked pair: a muted video item on Main and an
/// audio item on the Audio track at the same time, sharing one new `link` value, so they edit together and the sound
/// can be cut apart (J and L cuts) with the link override.
fn import_ops(doc: &ove_core::Document, probed: Vec<Probed>, now: i64) -> Vec<Op> {
    let track_end = |track: &str| doc.items_on_track(track).iter().map(|i| i.end).max().unwrap_or(0);
    let (mut main_end, mut audio_end) = (track_end("t_main"), track_end("t_audio"));
    let has_audio_track = doc.track("t_audio").is_some();
    let t = ove_core::time::micros_to_json;
    let mut ops = Vec::new();
    for (n, f) in probed.into_iter().enumerate() {
        ops.push(Op::AddMedia { media: f.media, entry: Some(f.entry) });
        let id = format!("i_{:x}_{n}", now);
        if !f.video {
            audio_end += f.duration_us;
            let at = audio_end - f.duration_us;
            ops.push(Op::AddItem {
                item: json!({"id": id, "trackId": "t_audio", "type": "audio", "startTime": t(at),
                    "endTime": t(at + f.duration_us), "name": f.name, "mediaItemId": f.id}),
            });
            continue;
        }
        // Linked sound goes on the Audio track at the picture's time; the pair starts after both tracks' ends.
        let linked = f.audio && has_audio_track;
        let at = if linked { main_end.max(audio_end) } else { main_end };
        main_end = at + f.duration_us;
        let mut video = json!({"id": id, "trackId": "t_main", "type": "video", "startTime": t(at),
            "endTime": t(main_end), "name": f.name, "mediaItemId": f.id});
        if linked {
            let link = format!("l_{:x}_{n}", now);
            audio_end = main_end;
            video["muted"] = true.into();
            video["link"] = link.clone().into();
            ops.push(Op::AddItem { item: video });
            ops.push(Op::AddItem {
                item: json!({"id": format!("{id}_a"), "trackId": "t_audio", "type": "audio", "startTime": t(at),
                    "endTime": t(main_end), "name": f.name, "mediaItemId": f.id, "link": link}),
            });
        } else {
            ops.push(Op::AddItem { item: video });
        }
    }
    ops
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
    let count = probed.len();
    let ops = import_ops(p.doc(), probed, now);
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
    let thumbs: Vec<(String, String, PathBuf, i64)> = probed
        .iter()
        .filter(|f| f.video)
        .map(|f| {
            let key = f.entry.get("contentId").and_then(Value::as_str).unwrap_or(&f.id).to_string();
            (f.id.clone(), key, f.path.clone(), f.duration_us)
        })
        .collect();
    let b = bundle.clone();
    let v = serial(&app, "import_media", move |app, state| add_probed(app, state, &b, probed, now, source)).await?;
    // The files are listed now; thumbnails follow as their tasks finish.
    queue_thumbnails(&app, &bundle, thumbs);
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
        if let Some(paths) = crate::scripted_dialogs::open() {
            let files: Vec<tauri_plugin_dialog::FilePath> = paths.into_iter().map(Into::into).collect();
            return Ok((!files.is_empty()).then_some(files));
        }
        Ok(a.dialog().file().add_filter("Media", &exts).blocking_pick_files())
    })
    .await?;
    let Some(files) = files else { return Ok(None) };
    let paths: Vec<String> =
        files.into_iter().filter_map(|f| f.into_path().ok()).map(|p| p.display().to_string()).collect();
    import_media(app, paths).await.map(Some)
}

/// Moves an item to a new start time (and optionally another track), keeping its duration. Linked items move with
/// it unless `linked` is false (Alt in the timeline). Landing on another item is allowed (the overlap plays as a
/// crossfade).
#[tauri::command]
#[specta::specta]
pub async fn move_item(
    app: AppHandle,
    id: String,
    start: f64,
    track_id: Option<String>,
    linked: Option<bool>,
) -> AppResult<ProjectView> {
    let start = ove_core::time::secs_to_micros(start.max(0.0))
        .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "The time is invalid."))?;
    serial(&app, "move_item", move |app, state| {
        let linked = linked.unwrap_or(true);
        let req = ove_edit::Request::Move { item: id, start, track: track_id, linked, crossfade: true };
        crate::edits::plan_and_apply(app, state, req)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_items(app: AppHandle, ids: Vec<String>) -> AppResult<ProjectView> {
    // Linked items go with the items deleted.
    serial(&app, "remove_items", move |app, state| {
        crate::edits::plan_and_apply(app, state, ove_edit::Request::Delete { items: ids, linked: true })
    })
    .await
}

/// Splits a clip at a timeline time (seconds): the left part keeps the id, the right part is a new item. Linked
/// items are split at the same time. Speed, reverse playback, keyframes, caption segments, fades and transitions are
/// carried over so both parts look and sound exactly like the uncut clip.
#[tauri::command]
#[specta::specta]
pub async fn split_item(app: AppHandle, id: String, at: f64) -> AppResult<ProjectView> {
    let at = ove_core::time::secs_to_micros(at).unwrap_or(0);
    serial(&app, "split_item", move |app, state| {
        let req = ove_edit::Request::Split { items: vec![id], tracks: vec![], at, linked: true };
        crate::edits::plan_and_apply(app, state, req)
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
    /// The agent client that made it (an MCP client's name, such as claude-code), when recorded.
    pub client: Option<String>,
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
            client: h.actor.as_ref().and_then(|a| a.client.clone()),
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

    /// Import of 200 files: probes run as tasks and the files go into the project as one revision before any
    /// thumbnail exists; thumbnails then appear as their tasks finish.
    #[test]
    fn large_import_lists_every_file_at_once_and_thumbnails_follow() {
        let dir = tempfile::tempdir().unwrap();
        let source = crate::tasks::testing::video(dir.path(), "source.mp4");
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).unwrap();
        let paths: Vec<String> = (0..200)
            .map(|i| {
                let p = media.join(format!("clip_{i:03}.mp4"));
                std::fs::copy(&source, &p).unwrap();
                p.display().to_string()
            })
            .collect();
        let bundle = dir.path().join("Import.ove");
        let tasks = crate::tasks::testing::scheduler();
        let project = crate::tasks::project_key(&bundle);

        let t = std::time::Instant::now();
        let (probed, errors) = probed_records(crate::tasks::probe_all(&tasks, &project, &paths), 1);
        let probe_time = t.elapsed();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(probed.len(), 200);
        // Every entry carries a content id from the add-media op; copies of one file share it.
        let cids: BTreeSet<&str> = probed.iter().filter_map(|f| f.entry.get("contentId")?.as_str()).collect();
        assert_eq!(cids.len(), 1, "200 copies, one content");
        assert!(probed.iter().all(|f| f.entry.get("contentId").is_some()));
        // Thumbnails named by media id here, so the 200 copies make 200 tasks (the app shares one per content).
        let thumbs: Vec<(String, PathBuf, i64)> =
            probed.iter().filter(|f| f.video).map(|f| (f.id.clone(), f.path.clone(), f.duration_us)).collect();
        let empty = ove_core::Document::new("p", "Import", 1);
        let doc = ove_core::ops::apply_batch(&empty, &default_tracks()).unwrap().doc;
        let ops = import_ops(&doc, probed, 1);
        let after = ove_core::ops::apply_batch(&doc, &ops).unwrap().doc;
        assert_eq!(after.project.media.len(), 200, "every file listed in one revision");
        assert_eq!(after.items_on_track("t_main").len(), 200);
        let list_time = t.elapsed();
        assert!(!thumbs_dir(&bundle).exists(), "listed before any thumbnail exists");

        let made = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for (id, src, dur) in thumbs {
            let m = made.clone();
            let out = thumbs_dir(&bundle).join(format!("{id}.png"));
            crate::tasks::queue_thumbnail(&tasks, &project, src, out, dur, move |ok| {
                assert!(ok);
                m.fetch_add(1, Ordering::SeqCst);
            });
        }
        let first = made.load(Ordering::SeqCst);
        let w = tasks.wait(&ove_tasks::TaskFilter::default(), std::time::Duration::from_secs(120));
        assert!(w.done, "{w:?}");
        let thumb_time = t.elapsed() - list_time;
        assert!(first < 200, "thumbnails arrive after the list, as tasks finish ({first} were ready at once)");
        assert_eq!(made.load(Ordering::SeqCst), 200);
        let files = std::fs::read_dir(thumbs_dir(&bundle)).unwrap().count();
        assert_eq!(files, 200, "one PNG per file and no part files left");
        println!("200-file import: probes {probe_time:?}, listed at {list_time:?}, thumbnails took {thumb_time:?}");
    }

    /// A project from before content ids: the background check stores ids as one migration revision, older
    /// thumbnails and transcripts move onto them without being made again, and a relinked file keeps them.
    #[test]
    fn upgrade_keeps_derived_data_and_relink_reuses_it() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Old.ove");
        let source = crate::tasks::testing::video(dir.path(), "interview.mp4");
        let empty = ove_core::Document::new("p", "Old", 1);
        let mut doc = ove_core::ops::apply_batch(&empty, &default_tracks()).unwrap().doc;
        // The entry an older build wrote: size and time, no content id.
        let (stamp_size, stamp_time) = ove_store::media::file_stamp(&source).unwrap();
        let old_entry = json!({"mediaId": "m1", "nativePath": source.display().to_string(), "ownedByBundle": false,
            "probeStatus": "probed", "size": stamp_size, "modifiedAt": stamp_time.unwrap()});
        doc = ove_core::ops::apply_batch(
            &doc,
            &[Op::AddMedia { media: json!({"id": "m1", "data": {"type": "video"}}), entry: Some(old_entry) }],
        )
        .unwrap()
        .doc;
        // Its derived data: a thumbnail named by media id and a transcript under the path/size/time key.
        std::fs::create_dir_all(thumbs_dir(&bundle)).unwrap();
        make_thumbnail(&source, &thumbs_dir(&bundle).join("m1.png"), 600_000).unwrap();
        let transcript = ove_transcribe::Transcript {
            language: "en".into(),
            model: "base".into(),
            words: vec![],
            duration_us: 600_000,
        };
        ove_transcribe::cache::store(&bundle, &ove_transcribe::cache::key(&source, "base", None), &transcript);

        // Content ids are computed and stored.
        let ops = identity_ops(&bundle, &doc);
        assert_eq!(ops.len(), 1);
        let doc = ove_core::ops::apply_batch(&doc, &ops).unwrap().doc;
        let cid = doc.registry["m1"].fields["contentId"].as_str().unwrap().to_string();
        assert!(identity_ops(&bundle, &doc).is_empty(), "nothing more to compute");
        // Derived data moves onto the id.
        assert_eq!(migrate_derived(&bundle, &doc), 2);
        assert!(thumbs_dir(&bundle).join(format!("{cid}.png")).is_file());
        assert!(!thumbs_dir(&bundle).join("m1.png").exists());
        assert!(thumbnail_for(&bundle, &doc, "m1").is_some());
        let key = ove_transcribe::cache::content_key(&cid, "base", None);
        assert_eq!(ove_transcribe::cache::load(&bundle, &key), Some(transcript.clone()), "no new transcription");

        // The file moves to another folder and is relinked: same id, same derived data.
        let moved_dir = dir.path().join("moved");
        std::fs::create_dir_all(&moved_dir).unwrap();
        let moved = moved_dir.join("interview.mp4");
        std::fs::rename(&source, &moved).unwrap();
        let mut entry_set = Map::new();
        entry_set.insert("nativePath".into(), moved.display().to_string().into());
        entry_set.extend(ove_store::media::identity_fields(&moved));
        let doc = ove_core::ops::apply_batch(
            &doc,
            &[Op::UpdateMedia { id: "m1".into(), set: Map::new(), unset: vec![], entry_set, entry_unset: vec![] }],
        )
        .unwrap()
        .doc;
        assert_eq!(thumb_key(&doc, "m1"), cid);
        assert!(thumbnail_for(&bundle, &doc, "m1").is_some());
        assert_eq!(ove_transcribe::cache::load(&bundle, &key), Some(transcript));

        // A different file under the same name gets a new id: its derived data is rebuilt.
        std::fs::write(&moved, b"not the same video").unwrap();
        let ops = identity_ops(&bundle, &doc);
        let doc = ove_core::ops::apply_batch(&doc, &ops).unwrap().doc;
        assert_ne!(thumb_key(&doc, "m1"), cid);
        assert!(thumbnail_for(&bundle, &doc, "m1").is_none());
    }

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
    fn importing_a_camera_clip_makes_a_linked_pair() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let mut p = lib.create("Camera").unwrap();
        p.apply(default_tracks(), Source::Gui, None).unwrap();
        let probed = |id: &str, video: bool, audio: bool| Probed {
            media: json!({"id": id, "data": {"type": if video { "video" } else { "audio" }, "fileName": format!("{id}.mov"),
                "duration": 4, "hasAudio": audio}}),
            entry: json!({"mediaId": id, "nativePath": format!("/x/{id}.mov"), "ownedByBundle": false}),
            id: id.into(),
            name: format!("{id}.mov"),
            path: PathBuf::from(format!("/x/{id}.mov")),
            duration_us: 4_000_000,
            video,
            audio,
        };
        let ops = import_ops(p.doc(), vec![probed("cam", true, true), probed("silent", true, false)], 7);
        p.apply(ops, Source::Gui, None).unwrap();
        let doc = p.doc();
        let linked: Vec<_> = doc.project.items.values().filter(|i| i.fields.contains_key("link")).collect();
        assert_eq!(linked.len(), 2, "one link value on two items");
        assert_eq!(linked[0].fields["link"], linked[1].fields["link"]);
        let (v, a) = if linked[0].kind == "video" { (linked[0], linked[1]) } else { (linked[1], linked[0]) };
        assert_eq!((v.track_id.as_str(), a.track_id.as_str()), ("t_main", "t_audio"));
        assert_eq!((v.start, v.end), (a.start, a.end), "same timing");
        assert_eq!(v.fields["muted"], json!(true), "the sound plays once, from the audio item");
        // The silent clip is a single video item after the pair.
        let silent = doc.project.items.values().find(|i| i.media_id() == Some("silent")).unwrap();
        assert!(!silent.fields.contains_key("link"));
        assert_eq!(silent.start, 4_000_000);
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
