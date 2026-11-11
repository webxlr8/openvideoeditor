//! Bridge commands. Every command returns `Result<T, AppError>`; none may panic across the
//! bridge (work that could panic runs inside [`crate::worker::contain`]).

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::media::{self, MediaInfo};
use crate::transport::{PlaybackState, TransportCommand};
use crate::viewer::{ViewerBackend, ViewerMsg, ViewerRect, ViewerStats};

/// Static facts about the running app.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub gpu_adapter: Option<String>,
    pub decode_capabilities: String,
    /// Presentation path the viewer is using now (may change once; see ViewerBackendChanged).
    pub viewer_backend: ViewerBackend,
    pub log_dir: Option<String>,
}

/// Everything the diagnostics overlay shows.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub decode_path: String,
    pub viewer: ViewerStats,
    pub dropped_frames: u32,
    /// Audio/video offset in milliseconds; None while there is no audio clock.
    pub av_offset_ms: Option<f64>,
    pub process_memory_bytes: Option<f64>,
    pub audio_available: bool,
}

fn lock<'a, T>(m: &'a Mutex<T>, what: &str) -> AppResult<std::sync::MutexGuard<'a, T>> {
    m.lock().map_err(|_| AppError::new(ErrorCode::Internal, format!("internal state ({what}) is unavailable")))
}

#[tauri::command]
#[specta::specta]
pub fn app_info(state: State<'_, AppState>) -> AppResult<AppInfo> {
    let os = os_info::get();
    let viewer_backend = lock(&state.viewer_stats, "viewer")?.backend.unwrap_or(ViewerBackend::Stream);
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        os: format!("{} {}", os.os_type(), os.version()),
        arch: std::env::consts::ARCH.into(),
        gpu_adapter: lock(&state.gpu_summary, "gpu")?.clone(),
        decode_capabilities: crate::DECODE_CAPABILITIES.into(),
        viewer_backend,
        log_dir: state.log_dir.as_ref().map(|d| d.display().to_string()),
    })
}

/// Paths the app was launched with (or that arrived before the UI was ready). Marks the UI ready:
/// later paths arrive as `OpenPathsRequested` events.
#[tauri::command]
#[specta::specta]
pub fn take_pending_open_paths(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    let mut pending = lock(&state.pending_paths, "open queue")?;
    state.ui_ready.store(true, Ordering::SeqCst);
    Ok(std::mem::take(&mut *pending))
}

/// Opens a media file in the viewer. On failure nothing changes (the current file keeps playing).
#[tauri::command]
#[specta::specta]
pub async fn open_media(state: State<'_, AppState>, path: String) -> AppResult<MediaInfo> {
    let p = PathBuf::from(&path);
    crate::engine::precheck(&p).inspect_err(|e| tracing::warn!(path, code = ?e.code, "open refused: {}", e.message))?;
    let (reply, rx) = std::sync::mpsc::channel();
    state
        .engine
        .tx
        .send(crate::engine::EngineMsg::Open(p, reply))
        .map_err(|_| AppError::new(ErrorCode::Internal, "The playback engine is not running."))?;
    let info = tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(std::time::Duration::from_secs(30)))
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
        .map_err(|_| AppError::new(ErrorCode::Internal, "Opening the file timed out."))??;
    tracing::info!(path, codec = info.video_codec, "media opened");
    Ok(info)
}

#[tauri::command]
#[specta::specta]
pub async fn open_media_dialog(app: AppHandle) -> AppResult<Option<MediaInfo>> {
    let mut exts: Vec<&str> = media::VIDEO_EXTENSIONS.to_vec();
    exts.extend_from_slice(media::AUDIO_EXTENSIONS);
    let picked = app.dialog().file().add_filter("Media", &exts).blocking_pick_file();
    let Some(file) = picked else { return Ok(None) };
    let path = file
        .into_path()
        .map_err(|e| AppError::new(ErrorCode::InvalidArgument, format!("Unsupported file location: {e}")))?;
    let state = app.state::<AppState>();
    open_media(state, path.display().to_string()).await.map(Some)
}

#[tauri::command]
#[specta::specta]
pub fn close_media(state: State<'_, AppState>) -> AppResult<PlaybackState> {
    let _ = state.engine.tx.send(crate::engine::EngineMsg::Close);
    let mut snap = lock(&state.engine.shared.state, "playback")?.clone();
    snap.has_media = false;
    snap.playing = false;
    Ok(snap)
}

#[tauri::command]
#[specta::specta]
pub fn set_viewer_rect(app: AppHandle, state: State<'_, AppState>, rect: ViewerRect) -> AppResult<()> {
    let valid = [rect.x, rect.y, rect.width, rect.height, rect.scale_factor].iter().all(|v| v.is_finite());
    if !valid || rect.width < 0.0 || rect.height < 0.0 || rect.scale_factor <= 0.0 {
        return Err(AppError::new(ErrorCode::InvalidArgument, "The viewer rectangle is invalid."));
    }
    let layer = state.layer.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(guard) = layer.lock()
            && let Some(layer) = guard.as_ref()
        {
            layer.apply_rect(&rect);
        }
    });
    state.send_viewer(ViewerMsg::Rect(rect));
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn transport(state: State<'_, AppState>, command: TransportCommand) -> AppResult<PlaybackState> {
    if matches!(
        command,
        TransportCommand::SeekFrame { .. }
            | TransportCommand::Step { .. }
            | TransportCommand::GoToStart
            | TransportCommand::GoToEnd
    ) {
        *lock(&state.seek_started, "seek timer")? = Some(Instant::now());
    }
    state
        .engine
        .tx
        .send(crate::engine::EngineMsg::Transport(command))
        .map_err(|_| AppError::new(ErrorCode::Internal, "The playback engine is not running."))?;
    // The engine applies the command on its next turn and emits `playbackChanged`; this is the state before it.
    Ok(lock(&state.engine.shared.state, "playback")?.clone())
}

#[tauri::command]
#[specta::specta]
pub fn set_volume(state: State<'_, AppState>, volume: f64, muted: bool) -> AppResult<PlaybackState> {
    let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
    let _ = state.engine.tx.send(crate::engine::EngineMsg::Volume(volume, muted));
    let mut snap = lock(&state.engine.shared.state, "playback")?.clone();
    snap.volume = volume;
    snap.muted = muted;
    Ok(snap)
}

#[tauri::command]
#[specta::specta]
pub fn playback_state(state: State<'_, AppState>) -> AppResult<PlaybackState> {
    Ok(lock(&state.engine.shared.state, "playback")?.clone())
}

#[tauri::command]
#[specta::specta]
pub fn diagnostics(state: State<'_, AppState>) -> AppResult<Diagnostics> {
    let viewer = lock(&state.viewer_stats, "viewer")?.clone();
    let has_media = lock(&state.engine.shared.media, "media")?.is_some();
    let audio_available = lock(&state.engine.shared.state, "playback")?.audio_available;
    let d = lock(&state.engine.shared.diag, "diagnostics")?.clone();
    Ok(Diagnostics {
        decode_path: if has_media { d.decode_path } else { "none".into() },
        viewer,
        dropped_frames: d.dropped_frames.min(u32::MAX as u64) as u32,
        av_offset_ms: d.av_offset_ms,
        process_memory_bytes: memory_stats::memory_stats().map(|m| m.physical_mem as f64),
        audio_available,
    })
}

/// Diagnostics hook: destroys the GPU device; the viewer must recover on its own.
#[tauri::command]
#[specta::specta]
pub fn simulate_device_loss(state: State<'_, AppState>) -> AppResult<()> {
    state.send_viewer(ViewerMsg::LoseDevice);
    Ok(())
}

/// Diagnostics hook: panics inside a background worker and returns the contained error.
#[tauri::command]
#[specta::specta]
pub fn debug_panic_worker() -> AppResult<()> {
    let handle =
        crate::worker::spawn_contained("a test worker", || -> () { panic!("forced worker panic (diagnostics)") })
            .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(AppError::new(ErrorCode::WorkerPanic, "A background task failed.")),
    }
}

/// Diagnostics hook: panics inside the viewer thread; the error arrives as `AppErrorRaised`.
#[tauri::command]
#[specta::specta]
pub fn debug_panic_viewer(state: State<'_, AppState>) -> AppResult<()> {
    state.send_viewer(ViewerMsg::Panic);
    Ok(())
}
