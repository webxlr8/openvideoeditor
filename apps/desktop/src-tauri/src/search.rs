//! Footage search in the app (add-semantic-media-search): background indexing of the open project's media, run as P3
//! tasks whose image encoder runs in the shared model worker (`ove model-worker`, started on demand, exits when
//! idle). Nothing is indexed and nothing is downloaded while the visual search model is not installed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ove_core::model::Document;
use ove_models::worker::WorkerClient;
use ove_vision::indexer::MediaKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, lock, serial};

/// The model worker shared by indexing and the search tools of this app: the `ove` beside the app serves it.
pub fn worker() -> Arc<WorkerClient> {
    ove_tools::tools::search::shared_worker()
}

/// Content ids already queued in this run of the app.
fn queued() -> &'static Mutex<BTreeSet<String>> {
    static QUEUED: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    QUEUED.get_or_init(Default::default)
}

/// The media of `doc` that can be indexed: online videos and images with a content id, as (content id, file, kind,
/// name).
pub fn indexable(bundle: &Path, doc: &Document, online: &BTreeSet<&str>) -> Vec<(String, PathBuf, MediaKind, String)> {
    doc.registry
        .values()
        .filter(|e| online.contains(e.media_id.as_str()))
        .filter_map(|e| {
            let cid = e.fields.get("contentId")?.as_str()?.to_string();
            let item = doc.project.media.get(&e.media_id)?;
            let data = item.fields.get("data")?;
            let kind = match data.get("type").and_then(Value::as_str)? {
                "video" => MediaKind::Video,
                "image" => MediaKind::Image,
                _ => return None,
            };
            let name = item
                .fields
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| item.file_name())
                .unwrap_or(&e.media_id)
                .to_string();
            Some((cid, ove_store::media::resolve_media_path(bundle, &e.native_path), kind, name))
        })
        .collect()
}

/// Queues indexing of the given media when the visual search model is installed; otherwise does nothing (the media
/// panel offers the install with its size). Each content is queued once per run; a finished index is reused.
pub fn queue_indexing(app: &AppHandle, bundle: &Path, items: Vec<(String, PathBuf, MediaKind, String)>) {
    let store = crate::models::store(app);
    // Both the model and the inference runtime must be installed; neither is ever downloaded here.
    if ove_vision::model::installed(&store).is_err() || ove_models::runtime::library_path(&store).is_err() {
        return;
    }
    let state = tauri::Manager::state::<crate::AppState>(app);
    let project = crate::tasks::project_key(bundle);
    for (cid, source, kind, name) in items {
        let fresh = queued().lock().map(|mut q| q.insert(cid.clone())).unwrap_or(false);
        if !fresh {
            continue;
        }
        let job = ove_vision::task::Job {
            project: project.clone(),
            bundle: bundle.to_path_buf(),
            source,
            content_id: cid.clone(),
            kind,
            name,
        };
        let root = store.root().to_path_buf();
        ove_vision::task::submit(
            &state.tasks,
            job,
            move || ove_vision::model::installed_encoder(&ove_models::Store::open(root), worker()),
            move |r| {
                if let Err(e) = r {
                    tracing::warn!(content = %cid, "media index: {e}");
                    if let Ok(mut q) = queued().lock() {
                        q.remove(&cid);
                    }
                }
            },
        );
    }
}

/// What the media panel's index pill shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaIndexStatus {
    /// The visual search model is installed.
    pub model_installed: bool,
    /// Its download size when it is not.
    pub download_bytes: f64,
    /// What the install offer names: the model, its weights' licence and where it downloads from.
    pub model_name: String,
    pub licence: String,
    pub host: String,
    /// Videos and images with a current index, of all.
    pub indexed: u32,
    pub total: u32,
    /// Indexing tasks queued or running for this project.
    pub running: u32,
}

fn bad_args() -> AppError {
    AppError::new(ErrorCode::InvalidArgument, "The arguments must be a JSON object.")
}

/// Searches the open project's media (`search_media`): runs off the project turn, so a first query that starts the
/// model worker does not hold up editing.
#[tauri::command]
#[specta::specta]
pub async fn media_search(app: AppHandle, args: String) -> AppResult<String> {
    crate::worker::blocking("media_search", move || {
        let a = match serde_json::from_str::<Value>(&args) {
            Ok(Value::Object(m)) => m,
            _ => return Err(bad_args()),
        };
        let state = tauri::Manager::state::<crate::AppState>(&app);
        Ok(crate::transcript::run(&app, &state, "search_media", &a)?.0.to_string())
    })
    .await
}

/// Organises media (`organize_media`): folders, tags, renames and subclips, as one revision.
#[tauri::command]
#[specta::specta]
pub async fn media_organize(app: AppHandle, ops: String) -> AppResult<ProjectView> {
    serial(&app, "media_organize", move |app, state| {
        let ops: Value = serde_json::from_str(&ops).map_err(|_| bad_args())?;
        let mut a = Map::new();
        a.insert("ops".into(), ops);
        let (_, view) = crate::transcript::run(app, state, "organize_media", &a)?;
        match view {
            Some(v) => Ok(v),
            None => {
                let g = lock(&state.projects.open)?;
                let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
                Ok(crate::projects::view(p))
            }
        }
    })
    .await
}

/// The index state of the open project: model installed or its size, indexed files of all, indexing tasks.
#[tauri::command]
#[specta::specta]
pub async fn media_index_status(app: AppHandle) -> AppResult<MediaIndexStatus> {
    crate::worker::blocking("media_index_status", move || {
        let state = tauri::Manager::state::<crate::AppState>(&app);
        let (bundle, doc) = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
            (p.bundle().to_path_buf(), p.doc().clone())
        };
        let store = crate::models::store(&app);
        let manifest = ove_models::manifest::siglip2_manifest();
        let (installed, download, prov) = match ove_vision::model::installed(&store) {
            Ok(f) => (true, 0.0, Some(ove_vision::model::provenance_of(&f))),
            Err(n) => (false, n.download_bytes as f64, None),
        };
        let all: BTreeSet<&str> = doc.project.media.keys().map(String::as_str).collect();
        let items = indexable(&bundle, &doc, &all);
        // A model installed since the project opened starts indexing here (each content is queued once).
        if installed {
            queue_indexing(&app, &bundle, items.clone());
        }
        let indexed = items
            .iter()
            .filter(|(cid, ..)| {
                prov.as_ref().is_some_and(|p| {
                    matches!(ove_vision::index::lookup(&bundle, cid, Some(p)), ove_vision::index::Lookup::Ready(_))
                })
            })
            .count();
        let filter = ove_tasks::TaskFilter {
            project: Some(crate::tasks::project_key(&bundle)),
            kind: Some(ove_vision::task::KIND.into()),
        };
        Ok(MediaIndexStatus {
            model_installed: installed,
            download_bytes: download,
            model_name: ove_vision::model::MODEL_NAME.into(),
            licence: manifest.licence.clone(),
            host: manifest.host().unwrap_or_default().to_string(),
            indexed: indexed as u32,
            total: items.len() as u32,
            running: state.tasks.list(&filter).len() as u32,
        })
    })
    .await
}
