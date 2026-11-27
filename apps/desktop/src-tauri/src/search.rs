//! Footage search in the app (add-semantic-media-search): background indexing of the open project's media, run as P3
//! tasks whose image encoder runs in the shared model worker (`ove model-worker`, started on demand, exits when
//! idle). Nothing is indexed and nothing is downloaded while the visual search model is not installed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ove_core::model::Document;
use ove_models::worker::WorkerClient;
use ove_vision::indexer::MediaKind;
use serde_json::Value;
use tauri::AppHandle;

/// The model worker shared by every search task of this app.
pub fn worker() -> Arc<WorkerClient> {
    static WORKER: OnceLock<Arc<WorkerClient>> = OnceLock::new();
    WORKER.get_or_init(|| Arc::new(WorkerClient::new(ove_program(), &["model-worker"]))).clone()
}

/// The `ove` program next to the app's own program (it serves `ove model-worker`).
fn ove_program() -> PathBuf {
    let name = if cfg!(windows) { "ove.exe" } else { "ove" };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
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
            let name = item.fields.get("name").and_then(Value::as_str).unwrap_or(&e.media_id).to_string();
            Some((cid, ove_store::media::resolve_media_path(bundle, &e.native_path), kind, name))
        })
        .collect()
}

/// Queues indexing of the given media when the visual search model is installed; otherwise does nothing (the media
/// panel offers the install with its size). Each content is queued once per run; a finished index is reused.
pub fn queue_indexing(app: &AppHandle, bundle: &Path, items: Vec<(String, PathBuf, MediaKind, String)>) {
    let store = crate::models::store(app);
    if ove_vision::model::installed(&store).is_err() {
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
