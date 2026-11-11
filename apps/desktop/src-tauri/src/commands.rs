//! Bridge commands. Every command returns `Result<T, AppError>`; none may panic across the
//! bridge (work that could panic runs inside [`crate::worker::contain`]).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::media::{self, MediaInfo};
use crate::transport::{PlaybackState, Transport, TransportCommand};
use crate::viewer::{ViewerBackend, ViewerMsg, ViewerRect, ViewerStats};
use crate::{AppState, PlaybackChanged};

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

/// Sends the frame at the transport's position to the viewer and notifies the UI.
fn show_current(app: &AppHandle, state: &PlaybackState, viewer: &Sender<ViewerMsg>) {
    if state.has_media && state.total_frames > 0 {
        let frame = media::test_pattern_frame(state.frame, state.total_frames, state.frame_rate);
        let _ = viewer.send(ViewerMsg::Frame(frame));
    }
    let _ = PlaybackChanged(state.clone()).emit(app);
}

/// Advances playback in real time while playing.
pub fn spawn_playback_thread(
    app: AppHandle,
    transport: Arc<(Mutex<Transport>, Condvar)>,
    viewer: Sender<ViewerMsg>,
) -> std::io::Result<()> {
    crate::worker::spawn_contained("playback", move || {
        let (lock, cvar) = &*transport;
        let mut next = Instant::now();
        loop {
            let Ok(guard) = lock.lock() else { return };
            if !guard.state.playing {
                let Ok((g, _)) = cvar.wait_timeout(guard, std::time::Duration::from_millis(250)) else { return };
                drop(g);
                next = Instant::now();
                continue;
            }
            let interval = guard.tick_interval();
            drop(guard);
            next += interval;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now;
            }
            let Ok(mut guard) = lock.lock() else { return };
            let was_playing = guard.state.playing;
            let changed = guard.tick();
            let snapshot = guard.state.clone();
            drop(guard);
            if changed || (was_playing && !snapshot.playing) {
                show_current(&app, &snapshot, &viewer);
            }
        }
    })?;
    Ok(())
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
pub fn open_media(app: AppHandle, state: State<'_, AppState>, path: String) -> AppResult<MediaInfo> {
    let info = media::open(&PathBuf::from(&path)).inspect_err(|e| {
        tracing::warn!(path, code = ?e.code, "open failed: {}", e.message);
    })?;
    let (lock_t, cvar) = &*state.transport;
    let snapshot = {
        let mut t = lock(lock_t, "transport")?;
        t.load(info.total_frames, info.frame_rate);
        t.state.clone()
    };
    cvar.notify_all();
    *lock(&state.media, "media")? = Some(info.clone());
    tracing::info!(path, placeholder = info.placeholder, "media opened");
    show_current(&app, &snapshot, &lock(&state.viewer_tx, "viewer")?.clone());
    Ok(info)
}

/// Shows the system file dialog and opens the chosen file. `None` when cancelled.
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
    open_media(app.clone(), state, path.display().to_string()).map(Some)
}

#[tauri::command]
#[specta::specta]
pub fn close_media(app: AppHandle, state: State<'_, AppState>) -> AppResult<PlaybackState> {
    let snapshot = {
        let mut t = lock(&state.transport.0, "transport")?;
        t.load(0, 30.0);
        t.state.clone()
    };
    *lock(&state.media, "media")? = None;
    let _ = PlaybackChanged(snapshot.clone()).emit(&app);
    Ok(snapshot)
}

/// The viewer element's rectangle, measured by the UI after every layout change.
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
pub fn transport(app: AppHandle, state: State<'_, AppState>, command: TransportCommand) -> AppResult<PlaybackState> {
    let (lock_t, cvar) = &*state.transport;
    let (before, snapshot) = {
        let mut t = lock(lock_t, "transport")?;
        let before = t.state.frame;
        t.apply(command);
        (before, t.state.clone())
    };
    cvar.notify_all();
    if snapshot.frame != before {
        *lock(&state.seek_started, "seek timer")? = Some(Instant::now());
    }
    show_current(&app, &snapshot, &lock(&state.viewer_tx, "viewer")?.clone());
    Ok(snapshot)
}

#[tauri::command]
#[specta::specta]
pub fn set_volume(app: AppHandle, state: State<'_, AppState>, volume: f64, muted: bool) -> AppResult<PlaybackState> {
    let snapshot = {
        let mut t = lock(&state.transport.0, "transport")?;
        t.set_volume(volume, muted);
        t.state.clone()
    };
    let _ = PlaybackChanged(snapshot.clone()).emit(&app);
    Ok(snapshot)
}

#[tauri::command]
#[specta::specta]
pub fn playback_state(state: State<'_, AppState>) -> AppResult<PlaybackState> {
    Ok(lock(&state.transport.0, "transport")?.state.clone())
}

#[tauri::command]
#[specta::specta]
pub fn diagnostics(state: State<'_, AppState>) -> AppResult<Diagnostics> {
    let viewer = lock(&state.viewer_stats, "viewer")?.clone();
    let has_media = lock(&state.media, "media")?.is_some();
    let audio_available = lock(&state.transport.0, "transport")?.state.audio_available;
    Ok(Diagnostics {
        decode_path: if has_media { "test pattern (CPU)".into() } else { "none".into() },
        viewer,
        dropped_frames: 0,
        av_offset_ms: None,
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
