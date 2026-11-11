//! Timeline preview and export commands: play the composited timeline, export it to MP4 in the background with
//! progress events and cancellation, pick a destination, reveal the result.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use crate::AppState;
use crate::engine::EngineMsg;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::media::MediaInfo;

/// What to export.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub path: String,
    /// Output height in pixels (720, 1080, 2160, …).
    pub height: u32,
    pub fps: u32,
}

/// Export progress (and the end of an export: `done`, with `error` or `path`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub fraction: f64,
    pub frame: u32,
    pub total_frames: u32,
    pub done: bool,
    pub error: Option<String>,
    pub path: Option<String>,
    pub encoder: Option<String>,
    pub seconds_taken: Option<f64>,
}

fn cancel_flag() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static F: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(None))
}

fn open_doc(state: &AppState) -> AppResult<(Arc<ove_core::Document>, PathBuf)> {
    let g =
        state.projects.open.lock().map_err(|_| AppError::new(ErrorCode::Internal, "internal state is unavailable"))?;
    let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open a project first."))?;
    Ok((Arc::new(p.doc().clone()), p.bundle().to_path_buf()))
}

/// Tells the engine about a changed document (called after every project edit).
pub fn notify_doc(state: &AppState, doc: &ove_core::Document, bundle: &std::path::Path) {
    let _ = state.engine.tx.send(EngineMsg::UpdateDoc(Arc::new(doc.clone()), bundle.to_path_buf()));
}

/// Plays the open project's composited timeline in the viewer.
#[tauri::command]
#[specta::specta]
pub async fn open_timeline(state: State<'_, AppState>) -> AppResult<MediaInfo> {
    let (doc, bundle) = open_doc(&state)?;
    let (reply, rx) = std::sync::mpsc::channel();
    state
        .engine
        .tx
        .send(EngineMsg::OpenTimeline(doc, bundle, reply))
        .map_err(|_| AppError::new(ErrorCode::Internal, "The playback engine is not running."))?;
    tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(std::time::Duration::from_secs(10)))
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
        .map_err(|_| AppError::new(ErrorCode::Internal, "The timeline did not start in time."))?
}

/// Starts exporting the open project in the background. Progress arrives as `exportProgress` events.
#[tauri::command]
#[specta::specta]
pub fn export_project(app: AppHandle, state: State<'_, AppState>, request: ExportRequest) -> AppResult<()> {
    let (doc, bundle) = open_doc(&state)?;
    let mut slot = cancel_flag().lock().map_err(|_| AppError::new(ErrorCode::Internal, "export state unavailable"))?;
    if slot.as_ref().is_some_and(|c| !c.load(Ordering::SeqCst)) {
        return Err(AppError::new(ErrorCode::InvalidArgument, "An export is already running."));
    }
    if request.path.trim().is_empty() {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Choose where to save the video."));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *slot = Some(cancel.clone());
    drop(slot);
    let settings = ove_render::ExportSettings { height: request.height, fps: request.fps, ..Default::default() };
    let out = PathBuf::from(&request.path);
    crate::worker::spawn_contained("export", move || {
        let mut last = -1i64;
        let res = ove_render::export(&doc, Some(&bundle), &out, &settings, &cancel, |p| {
            let permille = (p.fraction * 1000.0) as i64;
            if permille != last {
                last = permille;
                let _ = ExportProgress {
                    fraction: p.fraction,
                    frame: p.frame as u32,
                    total_frames: p.total_frames as u32,
                    done: false,
                    error: None,
                    path: None,
                    encoder: None,
                    seconds_taken: None,
                }
                .emit(&app);
            }
        });
        // The export is over either way: a new one may start.
        cancel.store(true, Ordering::SeqCst);
        let end = match res {
            Ok(r) => {
                tracing::info!(path = r.path, encoder = r.video_encoder, seconds = r.seconds_taken, "export finished");
                ExportProgress {
                    fraction: 1.0,
                    frame: r.frames as u32,
                    total_frames: r.frames as u32,
                    done: true,
                    error: None,
                    path: Some(r.path),
                    encoder: Some(r.video_encoder),
                    seconds_taken: Some(r.seconds_taken),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "export did not finish");
                ExportProgress {
                    fraction: 0.0,
                    frame: 0,
                    total_frames: 0,
                    done: true,
                    error: Some(e.to_string()),
                    path: None,
                    encoder: None,
                    seconds_taken: None,
                }
            }
        };
        let _ = end.emit(&app);
    })
    .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn cancel_export() -> AppResult<()> {
    if let Ok(g) = cancel_flag().lock()
        && let Some(c) = g.as_ref()
    {
        c.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Save dialog for the export destination. `None` when cancelled.
#[tauri::command]
#[specta::specta]
pub async fn choose_export_path(app: AppHandle, default_name: String) -> AppResult<Option<String>> {
    let name = if default_name.to_lowercase().ends_with(".mp4") { default_name } else { format!("{default_name}.mp4") };
    let picked = app.dialog().file().add_filter("MP4 video", &["mp4"]).set_file_name(&name).blocking_save_file();
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()))
}

/// Shows a file in the system file manager.
#[tauri::command]
#[specta::specta]
pub fn reveal_path(path: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let r = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(&p).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(&p)).spawn()
    };
    r.map(|_| ()).map_err(|e| AppError::new(ErrorCode::Io, format!("Could not open the folder: {e}")))
}
