//! Timeline preview and export commands: play the composited timeline, export it to MP4 in the background with
//! progress events and cancellation, pick a destination, reveal the result.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use crate::AppState;
use crate::engine::EngineMsg;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::media::MediaInfo;
use crate::projects::lock;
use crate::worker::{self, relock};

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
    /// The video encoder in use (known once encoding starts).
    pub encoder: Option<String>,
    /// Set when the encoder is a fallback (not hardware, not H.264): show it to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub warning: Option<String>,
    pub seconds_taken: Option<f64>,
}

impl ExportProgress {
    fn failed(error: String) -> Self {
        ExportProgress {
            fraction: 0.0,
            frame: 0,
            total_frames: 0,
            done: true,
            error: Some(error),
            path: None,
            encoder: None,
            warning: None,
            seconds_taken: None,
        }
    }
}

/// The running export's cancel flag; `None` when no export is running.
fn running() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static F: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(None))
}

/// Ends an export when dropped, however its thread ends (result, error or panic): clears the "already running"
/// state and sends the final `done` event (an error one, unless a result was set).
struct ExportEnd<F: FnOnce(ExportProgress)> {
    cancel: Arc<AtomicBool>,
    end: Option<ExportProgress>,
    send: Option<F>,
}

impl<F: FnOnce(ExportProgress)> Drop for ExportEnd<F> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        {
            let mut slot = relock(running());
            if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &self.cancel)) {
                *slot = None;
            }
        }
        let end = self.end.take().unwrap_or_else(|| {
            ExportProgress::failed("The export stopped because of an internal error. Details are in the log.".into())
        });
        if let Some(send) = self.send.take() {
            send(end);
        }
    }
}

/// Marks an export as running; refused while another one is.
fn begin() -> AppResult<Arc<AtomicBool>> {
    let mut slot = relock(running());
    if slot.is_some() {
        return Err(AppError::new(ErrorCode::InvalidArgument, "An export is already running."));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *slot = Some(cancel.clone());
    Ok(cancel)
}

fn open_doc(state: &AppState) -> AppResult<(Arc<ove_core::Document>, PathBuf)> {
    let g = lock(&state.projects.open)?;
    let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open a project first."))?;
    Ok((Arc::new(p.doc().clone()), p.bundle().to_path_buf()))
}

/// Tells the engine about a changed document (called after every project edit).
pub fn notify_doc(state: &AppState, doc: &ove_core::Document, bundle: &std::path::Path) {
    let _ = state.engine.send(EngineMsg::UpdateDoc(Arc::new(doc.clone()), bundle.to_path_buf()));
}

/// Plays the open project's composited timeline in the viewer.
#[tauri::command]
#[specta::specta]
pub async fn open_timeline(app: AppHandle) -> AppResult<MediaInfo> {
    worker::blocking("open_timeline", move || {
        let state = app.state::<AppState>();
        let (doc, bundle) = open_doc(&state)?;
        state.engine.request(
            |reply| EngineMsg::OpenTimeline(doc, bundle, reply),
            Duration::from_secs(10),
            "starting the timeline",
        )
    })
    .await
}

/// Refuses an export that cannot succeed, before anything starts. Offline media gets its own error code.
fn check_export(
    doc: &ove_core::Document,
    bundle: &std::path::Path,
    settings: &ove_render::ExportSettings,
) -> AppResult<()> {
    ove_render::export::preflight(doc, Some(bundle), settings).map_err(|e| {
        let offline = !ove_render::export::offline_media(doc, Some(bundle)).is_empty();
        let text = e.to_string();
        let mut c = text.chars();
        let message: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
        AppError::new(if offline { ErrorCode::MediaOffline } else { ErrorCode::InvalidArgument }, format!("{message}."))
    })
}

/// Starts exporting the open project in the background. Progress arrives as `exportProgress` events; the last one
/// has `done` set. Refused up front when media is offline, placeholders are pending or an export is running.
#[tauri::command]
#[specta::specta]
pub async fn export_project(app: AppHandle, request: ExportRequest) -> AppResult<()> {
    worker::blocking("export_project", move || {
        if request.path.trim().is_empty() {
            return Err(AppError::new(ErrorCode::InvalidArgument, "Choose where to save the video."));
        }
        let (doc, bundle) = open_doc(&app.state::<AppState>())?;
        let settings = ove_render::ExportSettings { height: request.height, fps: request.fps, ..Default::default() };
        // Checks every media file: done here, off the main thread.
        check_export(&doc, &bundle, &settings)?;
        let cancel = begin()?;
        let out = PathBuf::from(&request.path);
        let a = app.clone();
        let mut finish = ExportEnd {
            cancel: cancel.clone(),
            end: None,
            send: Some(move |end: ExportProgress| {
                let _ = end.emit(&a);
            }),
        };
        let spawned = worker::spawn_contained("export", move || {
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
                        warning: ove_render::export::encoder_warning(&p.encoder),
                        encoder: Some(p.encoder),
                        seconds_taken: None,
                    }
                    .emit(&app);
                }
            });
            finish.end = Some(match res {
                Ok(r) => {
                    tracing::info!(
                        path = r.path,
                        encoder = r.video_encoder,
                        seconds = r.seconds_taken,
                        "export finished"
                    );
                    if r.encoder_warning.is_some() {
                        tracing::warn!(encoder = r.video_encoder, "exported with a fallback encoder");
                    }
                    ExportProgress {
                        fraction: 1.0,
                        frame: r.frames as u32,
                        total_frames: r.frames as u32,
                        done: true,
                        error: None,
                        path: Some(r.path),
                        encoder: Some(r.video_encoder),
                        warning: r.encoder_warning,
                        seconds_taken: Some(r.seconds_taken),
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "export did not finish");
                    ExportProgress::failed(e.to_string())
                }
            });
            // `finish` drops here, or while unwinding from a panic: the export is over either way.
            drop(finish);
        });
        // If the thread could not start, the closure (and `finish` in it) was dropped: the slot is clear again.
        spawned.map(|_| ()).map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn cancel_export() -> AppResult<()> {
    if let Some(c) = relock(running()).as_ref() {
        c.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Save dialog for the export destination. `None` when cancelled.
#[tauri::command]
#[specta::specta]
pub async fn choose_export_path(app: AppHandle, default_name: String) -> AppResult<Option<String>> {
    worker::blocking("choose_export_path", move || {
        let name =
            if default_name.to_lowercase().ends_with(".mp4") { default_name } else { format!("{default_name}.mp4") };
        let picked = app.dialog().file().add_filter("MP4 video", &["mp4"]).set_file_name(&name).blocking_save_file();
        Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()))
    })
    .await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_end_runs_on_panic_and_frees_the_slot() {
        let sent = Arc::new(Mutex::new(Vec::<ExportProgress>::new()));
        let cancel = begin().unwrap();
        assert!(begin().is_err(), "a second export is refused while one runs");
        let s2 = sent.clone();
        let finish =
            ExportEnd { cancel: cancel.clone(), end: None, send: Some(move |e: ExportProgress| relock(&s2).push(e)) };
        // The export thread panics before setting a result.
        let r = worker::contain("export", move || {
            let _finish = finish;
            panic!("injected export failure");
        });
        assert!(r.is_err());
        let sent = relock(&sent);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].done && sent[0].error.as_deref().is_some_and(|e| e.contains("internal error")));
        assert!(cancel.load(Ordering::SeqCst));
        // A new export may start.
        let again = begin().unwrap();
        drop(ExportEnd { cancel: again, end: None, send: None::<fn(ExportProgress)> });
        assert!(relock(running()).is_none());
    }
}
