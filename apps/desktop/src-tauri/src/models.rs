//! Settings → Storage, Models: the local machine-learning models in the shared model store (`ove-models`), with
//! their size, purpose and folder, and Remove. Removing a model a running job uses waits for the job.

use std::path::PathBuf;

use ove_models::Store;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

/// One model as the Storage list shows it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub id: String,
    pub purpose: String,
    pub licence: String,
    /// Size of the whole model in bytes.
    pub bytes: f64,
    pub installed: bool,
    /// The folder holding its files, when installed.
    pub location: Option<String>,
    /// A running job uses it (removal waits for the job).
    pub in_use: bool,
    /// Where it downloads from (shown before the first download).
    pub host: Option<String>,
    /// Bytes to download (an archive counts once).
    pub download_bytes: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalModels {
    /// The store folder.
    pub root: String,
    /// Free space on the store's disk in bytes, when known.
    pub free_bytes: Option<f64>,
    pub models: Vec<LocalModel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RemoveStatus {
    Removed,
    /// A running job uses the model; it is removed when the job ends (`localModelsChanged` follows).
    Waiting,
    NotInstalled,
}

/// The outcome of a removal, returned by `remove_local_model` and, after a wait, sent as an event.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelsChanged {
    pub id: String,
    pub status: RemoveStatus,
    pub freed_bytes: f64,
    pub free_bytes: Option<f64>,
}

fn store_root(app: &AppHandle) -> PathBuf {
    let data = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("openvideoeditor"));
    ove_transcribe::models::model_dir(&data)
}

pub(crate) fn store(app: &AppHandle) -> Store {
    Store::open(store_root(app))
}

fn free_bytes(store: &Store) -> Option<f64> {
    ove_models::space::free_bytes(store.root()).ok().map(|b| b as f64)
}

fn model_error(e: ove_models::ModelError) -> AppError {
    let code = match e {
        ove_models::ModelError::UnknownModel(_) => ErrorCode::InvalidArgument,
        _ => ErrorCode::Io,
    };
    AppError::new(code, e.to_string())
}

/// Every known model, with what is installed. Speech models saved by earlier versions are recorded on first look.
#[tauri::command]
#[specta::specta]
pub async fn local_models(app: AppHandle) -> AppResult<LocalModels> {
    worker::blocking("local_models", move || {
        let store = store(&app);
        store.adopt_root_files();
        let models = store
            .list()
            .into_iter()
            .map(|m| LocalModel {
                in_use: store.in_use(&m.id),
                id: m.id,
                purpose: m.purpose,
                licence: m.licence,
                bytes: m.bytes as f64,
                installed: m.installed,
                location: m.location.map(|p| p.display().to_string()),
                host: m.host,
                download_bytes: m.download_bytes as f64,
            })
            .collect();
        Ok(LocalModels { root: store.root().display().to_string(), free_bytes: free_bytes(&store), models })
    })
    .await
}

/// Deletes an installed model's files. When a running job uses it, returns `waiting` and removes it when the job
/// ends, then sends `localModelsChanged`.
#[tauri::command]
#[specta::specta]
pub async fn remove_local_model(app: AppHandle, id: String) -> AppResult<LocalModelsChanged> {
    worker::blocking("remove_local_model", move || {
        let store = store(&app);
        store.manifest(&id).map_err(model_error)?;
        let done = |store: &Store, removed: Option<u64>| LocalModelsChanged {
            id: id.clone(),
            status: if removed.is_some() { RemoveStatus::Removed } else { RemoveStatus::NotInstalled },
            freed_bytes: removed.unwrap_or(0) as f64,
            free_bytes: free_bytes(store),
        };
        if !store.in_use(&id) {
            let removed = store.remove(&id).map_err(model_error)?;
            return Ok(done(&store, removed));
        }
        let (app2, id2) = (app.clone(), id.clone());
        worker::spawn_contained("remove model when free", move || {
            let store = Store::open(store_root(&app2));
            let event = match store.remove_when_free(&id2, &mut || {}) {
                Ok(removed) => LocalModelsChanged {
                    id: id2.clone(),
                    status: if removed.is_some() { RemoveStatus::Removed } else { RemoveStatus::NotInstalled },
                    freed_bytes: removed.unwrap_or(0) as f64,
                    free_bytes: free_bytes(&store),
                },
                Err(e) => {
                    tracing::warn!("removing model {id2} failed: {e}");
                    LocalModelsChanged {
                        id: id2.clone(),
                        status: RemoveStatus::NotInstalled,
                        freed_bytes: 0.0,
                        free_bytes: None,
                    }
                }
            };
            let _ = event.emit(&app2);
        })
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(LocalModelsChanged { id, status: RemoveStatus::Waiting, freed_bytes: 0.0, free_bytes: free_bytes(&store) })
    })
    .await
}

/// Progress of a download the person started in Settings → Storage. The first event names the host the files come
/// from; the last has `finished` set, with `error` when it failed or was cancelled.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelInstallProgress {
    pub id: String,
    pub host: String,
    pub done_bytes: f64,
    pub total_bytes: f64,
    pub finished: bool,
    pub error: Option<String>,
}

type CancelFlags = std::sync::Mutex<Vec<(String, std::sync::Arc<std::sync::atomic::AtomicBool>)>>;

fn installs() -> std::sync::MutexGuard<'static, Vec<(String, std::sync::Arc<std::sync::atomic::AtomicBool>)>> {
    static INSTALLS: CancelFlags = std::sync::Mutex::new(Vec::new());
    INSTALLS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Downloads a model or the inference runtime after the person asked for it in Settings → Storage (this is their
/// consent). Returns at once; `localModelInstallProgress` reports the host and progress, and the end.
#[tauri::command]
#[specta::specta]
pub async fn install_local_model(app: AppHandle, id: String) -> AppResult<()> {
    worker::blocking("install_local_model", move || start_install(app, id)).await
}

fn start_install(app: AppHandle, id: String) -> AppResult<()> {
    let store = store(&app);
    let m = store.manifest(&id).map_err(model_error)?.clone();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut g = installs();
        if g.iter().any(|(k, _)| *k == id) {
            return Err(AppError::new(ErrorCode::InvalidArgument, format!("{id} is already downloading")));
        }
        g.push((id.clone(), cancel.clone()));
    }
    let host = m.host().unwrap_or_default().to_string();
    let total = m.download_bytes() as f64;
    let event = move |done: f64, finished: bool, error: Option<String>| LocalModelInstallProgress {
        id: m.id.clone(),
        host: host.clone(),
        done_bytes: done,
        total_bytes: total,
        finished,
        error,
    };
    let _ = event(0.0, false, None).emit(&app);
    let id2 = id.clone();
    worker::spawn_contained("install model", move || {
        let mut last = std::time::Instant::now();
        let result = store.ensure_installed(
            &id2,
            ove_models::Origin::Person,
            &mut |p| {
                if last.elapsed() >= std::time::Duration::from_millis(100) {
                    last = std::time::Instant::now();
                    let _ = event(p.done as f64, false, None).emit(&app);
                }
            },
            &cancel,
        );
        installs().retain(|(k, _)| *k != id2);
        let end = match result {
            Ok(_) => event(total, true, None),
            Err(e) => event(0.0, true, Some(e.to_string())),
        };
        let _ = end.emit(&app);
    })
    .map_err(|e| {
        installs().retain(|(k, _)| *k != id);
        AppError::new(ErrorCode::Internal, e.to_string())
    })?;
    Ok(())
}

/// Cancels a download started with `install_local_model`. Returns false when none was running.
#[tauri::command]
#[specta::specta]
pub async fn cancel_local_model_install(id: String) -> AppResult<bool> {
    worker::blocking("cancel_local_model_install", move || {
        let g = installs();
        let found = g.iter().find(|(k, _)| *k == id);
        if let Some((_, flag)) = found {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(found.is_some())
    })
    .await
}
