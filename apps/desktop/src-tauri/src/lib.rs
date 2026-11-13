//! Desktop shell: window, single instance and open-file routing, typed bridge, logs and crash
//! containment, the native viewer, and playback through ove-media / ove-playback (change 1).

pub mod assistant;
pub mod captions;
pub mod commands;
pub mod control;
pub mod edits;
pub mod engine;
pub mod error;
pub mod exporting;
pub mod fx;
pub mod generation;
pub mod interchange;
pub mod logging;
pub mod media;
pub mod motion;
pub mod projects;
pub mod scripted_dialogs;
pub mod support;
pub mod timeline;
pub mod transport;
pub mod viewer;
pub mod window_state;
pub mod worker;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Manager, RunEvent, WindowEvent};
use tauri_specta::{Event, collect_commands, collect_events};

use crate::error::AppError;
use crate::transport::PlaybackState;
use crate::viewer::{NativeLayer, ViewerBackend, ViewerMsg, ViewerStats};
use crate::worker::relock;

/// Playback state changed (frame, play/pause, speed, loop, volume).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct PlaybackChanged(pub PlaybackState);

/// Paths routed to the running app (second launch, Open With, Dock/taskbar drop, window drop).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct OpenPathsRequested {
    pub paths: Vec<String>,
}

/// A new streamed viewer frame is available at `ove-frame://localhost/latest` (candidate 3).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ViewerFrameReady {
    pub seq: u32,
    pub width: u32,
    pub height: u32,
}

/// The active viewer presentation path changed (e.g. native surface attached or fell back).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ViewerBackendChanged(pub ViewerBackend);

/// An error that happened outside a command (background task, routed file).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct AppErrorRaised(pub AppError);

/// Shared app state behind the commands.
pub struct AppState {
    pub engine: engine::EngineHandle,
    pub projects: projects::Projects,
    pub assistant: assistant::Assistant,
    pub viewer_tx: Mutex<Sender<ViewerMsg>>,
    pub viewer_stats: Arc<Mutex<ViewerStats>>,
    pub seek_started: Arc<Mutex<Option<Instant>>>,
    pub layer: Arc<Mutex<Option<Box<dyn NativeLayer>>>>,
    pub pending_paths: Mutex<Vec<String>>,
    pub ui_ready: AtomicBool,
    pub gpu_summary: Arc<Mutex<Option<String>>>,
    pub log_dir: Option<PathBuf>,
}

impl AppState {
    pub fn send_viewer(&self, msg: ViewerMsg) {
        let _ = relock(&self.viewer_tx).send(msg);
    }
}

/// What the decoder can do. Reported in the startup log line and app info.
#[cfg(target_os = "macos")]
pub const DECODE_CAPABILITIES: &str = "FFmpeg (LGPL); hardware: VideoToolbox, software fallback";
#[cfg(windows)]
pub const DECODE_CAPABILITIES: &str = "FFmpeg (LGPL); hardware: D3D11VA, software fallback";
#[cfg(not(any(target_os = "macos", windows)))]
pub const DECODE_CAPABILITIES: &str = "FFmpeg; software decode (hardware paths are macOS/Windows only)";

/// The bridge contract: every command and event the UI may use.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app_info,
            commands::take_pending_open_paths,
            commands::open_media,
            commands::open_media_dialog,
            commands::close_media,
            commands::set_viewer_rect,
            commands::transport,
            commands::set_volume,
            commands::playback_state,
            commands::diagnostics,
            commands::simulate_device_loss,
            commands::debug_panic_worker,
            commands::debug_panic_viewer,
            projects::list_projects,
            projects::create_project,
            projects::open_project,
            projects::close_project,
            projects::project_view,
            projects::import_media,
            projects::import_media_dialog,
            projects::move_item,
            projects::remove_items,
            projects::split_item,
            projects::set_aspect_ratio,
            projects::rename_project,
            projects::set_track_flag,
            projects::undo,
            projects::project_history,
            projects::restore_before,
            projects::redo,
            // Timeline exchange (FCPXML, XML, EDL, OTIO).
            control::report_ui_state,
            // Distribution: crash reports and update check (change 16).
            support::crash_reports,
            support::mark_crash_reports_seen,
            support::read_crash_report,
            support::check_for_updates,
            support::open_url,
            support::feedback_info,
            // AI generation with the user's provider keys (change 14).
            generation::generation_catalog,
            generation::set_provider_key,
            generation::clear_provider_key,
            generation::test_provider_key,
            generation::plan_generation,
            generation::start_generation,
            generation::generation_jobs,
            generation::cancel_generation,
            generation::list_voices,
            interchange::timeline_formats,
            interchange::export_timeline,
            interchange::import_timeline,
            // Timeline preview and export (render engine).
            exporting::open_timeline,
            exporting::export_project,
            exporting::cancel_export,
            exporting::choose_export_path,
            exporting::reveal_path,
            edits::item_detail,
            edits::update_item,
            edits::update_items,
            edits::add_items,
            edits::duplicate_items,
            edits::paste_items,
            edits::trim_item,
            edits::add_track,
            edits::rename_track,
            edits::reorder_tracks,
            edits::remove_track,
            assistant::agent_status,
            assistant::agent_set_key,
            assistant::agent_clear_key,
            assistant::agent_set_model,
            assistant::agent_chats,
            assistant::agent_chat,
            assistant::agent_send,
            assistant::agent_stop,
            assistant::agent_answer,
            assistant::agent_set_limits,
            assistant::agent_check_key,
            assistant::agent_usage,
            assistant::agent_undo_turn,
            // Captions: speech models, auto-captions, caption styles (change 10).
            captions::caption_models,
            captions::caption_presets,
            captions::download_caption_model,
            captions::auto_caption,
            captions::cancel_captions,
            captions::apply_caption_style,
            // Motion graphics and fonts (change 4 / 11).
            motion::list_fonts,
            motion::font_preview,
            motion::list_templates,
            motion::template_thumbnail,
            motion::check_code,
            // Effects, transitions, layouts and colour (change 12).
            fx::fx_catalog,
            fx::fx_preview,
            fx::import_lut,
            fx::match_color,
            fx::color_scopes,
        ])
        .events(collect_events![
            PlaybackChanged,
            OpenPathsRequested,
            ViewerFrameReady,
            ViewerBackendChanged,
            AppErrorRaised,
            projects::ProjectChanged,
            control::RemoteOpened,
            generation::GenerationUpdated,
            exporting::ExportProgress,
            assistant::AgentUiEvent,
            captions::CaptionJobProgress,
        ])
        .error_handling(tauri_specta::ErrorHandlingMode::Result)
}

/// Generated TypeScript for the bridge.
pub fn bindings_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/src/bindings.ts")
}

/// Renders the bindings to a string (via a temporary file).
pub fn render_bindings() -> Result<String, String> {
    let tmp =
        std::env::temp_dir().join(format!("ove-bindings-{}-{:?}.ts", std::process::id(), std::thread::current().id()));
    specta_builder()
        .export(
            specta_typescript::Typescript::default().header(
                "// Generated by `cargo run -p ove-desktop --features bindings --bin export-bindings`. Do not edit.\n",
            ),
            &tmp,
        )
        .map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&tmp).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    Ok(text)
}

/// Returns `Err` with a message when `committed` differs from freshly generated bindings.
pub fn check_bindings(committed: &str) -> Result<(), String> {
    let fresh = render_bindings()?;
    if fresh == committed {
        Ok(())
    } else {
        Err("ui/src/bindings.ts is stale: run `cargo run -p ove-desktop --features bindings --bin export-bindings`"
            .into())
    }
}

/// Keeps only arguments that look like file paths (drops flags and macOS process serial numbers),
/// resolved against `cwd`.
pub fn paths_from_args(args: &[String], cwd: &Path) -> Vec<String> {
    args.iter()
        .filter(|a| !a.starts_with('-') && !a.starts_with("psn_") && !a.is_empty())
        .map(|a| {
            let p = Path::new(a);
            if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) }
        })
        .map(|p| p.display().to_string())
        .collect()
}

fn focus_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn route_paths(app: &tauri::AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    tracing::info!(?paths, "open request routed");
    let state = app.state::<AppState>();
    focus_main(app);
    if state.ui_ready.load(Ordering::SeqCst) {
        let _ = OpenPathsRequested { paths }.emit(app);
    } else {
        relock(&state.pending_paths).extend(paths);
    }
}

fn state_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("window-state.json"))
}

fn restore_window(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    let monitors: Vec<window_state::Monitor> = {
        let primary = window.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
        window
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = m.name().cloned().unwrap_or_else(|| format!("display-{i}"));
                let wa = m.work_area();
                window_state::Monitor {
                    primary: primary.as_ref() == Some(&name),
                    name,
                    work_area: window_state::Rect {
                        x: wa.position.x,
                        y: wa.position.y,
                        width: wa.size.width,
                        height: wa.size.height,
                    },
                    scale: m.scale_factor(),
                }
            })
            .collect()
    };
    let saved = state_file(app).and_then(|p| window_state::load(&p));
    let placement = window_state::initial_placement(saved.as_ref(), &monitors);
    let r = placement.rect;
    let _ = window.set_size(tauri::PhysicalSize::new(r.width, r.height));
    let _ = window.set_position(tauri::PhysicalPosition::new(r.x, r.y));
    if placement.maximized {
        let _ = window.maximize();
    }
    tracing::info!(?placement, displays = monitors.len(), "window placed");
}

fn save_window(app: &tauri::AppHandle, window: &tauri::Window) {
    let Some(path) = state_file(app) else { return };
    let maximized = window.is_maximized().unwrap_or(false);
    let previous = window_state::load(&path);
    let (Ok(pos), Ok(size), Ok(Some(monitor))) =
        (window.outer_position(), window.inner_size(), window.current_monitor())
    else {
        return;
    };
    let rect = if maximized {
        previous.as_ref().map(|p| p.rect).unwrap_or_default()
    } else {
        window_state::Rect { x: pos.x, y: pos.y, width: size.width, height: size.height }
    };
    let state = window_state::SavedState {
        monitor: monitor.name().cloned().unwrap_or_default(),
        rect,
        scale: monitor.scale_factor(),
        maximized,
    };
    if let Err(e) = window_state::save(&path, &state) {
        tracing::warn!("could not save window state: {e}");
    }
}

/// Requested viewer layering: `OVE_VIEWER_LAYERING=below|above|stream` (default: below on
/// macOS/Windows, stream elsewhere).
pub fn requested_backend() -> ViewerBackend {
    match std::env::var("OVE_VIEWER_LAYERING").as_deref() {
        Ok("stream") => ViewerBackend::Stream,
        Ok("above") => ViewerBackend::NativeAbove,
        Ok("below") => ViewerBackend::NativeBelow,
        _ if cfg!(any(target_os = "macos", windows)) => ViewerBackend::NativeBelow,
        _ => ViewerBackend::Stream,
    }
}

/// The main window, from its `tauri.conf.json` entry (marked `"create": false` so it is made here).
fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main").cloned() else {
        return Ok(());
    };
    // A platform config (tauri.windows.conf.json, tauri.macos.conf.json) that forgets `"create": false` has already
    // made the window: keep it rather than fail to start.
    if app.get_webview_window("main").is_some() {
        tracing::warn!(
            "the main window was created from the config; mark it \"create\": false in every tauri.*.conf.json"
        );
        return Ok(());
    }
    #[allow(unused_mut, reason = "only the Windows debug build changes the builder")]
    let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?;
    // Debug builds driven by msedgedriver (the app end-to-end suite). The driver hands WebView2 its remote-debugging
    // switch and a profile folder in environment variables, but since WebView2 runtime 133 those are ignored when the
    // app passes browser arguments of its own, which wry always does (actions/runner-images#14738). So pass the
    // driver's values explicitly. Release builds never read the variables.
    #[cfg(all(windows, debug_assertions))]
    {
        if let Ok(extra) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            let args = webview2_args(&extra);
            tracing::info!(%args, "WebView2 arguments from the driver");
            builder = builder.additional_browser_args(&args);
        }
        if let Some(dir) = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").filter(|d| !d.is_empty()) {
            tracing::info!(dir = %std::path::Path::new(&dir).display(), "WebView2 profile folder from the driver");
            builder = builder.data_directory(std::path::PathBuf::from(dir));
        }
    }
    builder.build()?;
    Ok(())
}

/// wry's default WebView2 arguments together with `extra`, with a single merged `--disable-features` list (Chromium
/// keeps only the last one given).
#[cfg(any(test, all(windows, debug_assertions)))]
fn webview2_args(extra: &str) -> String {
    let mut features = vec!["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection"];
    let mut rest = Vec::new();
    for arg in extra.split_whitespace() {
        match arg.strip_prefix("--disable-features=") {
            Some(list) => {
                for f in list.split(',') {
                    if !f.is_empty() && !features.contains(&f) {
                        features.push(f);
                    }
                }
            }
            None => rest.push(arg),
        }
    }
    let mut out = format!("--disable-features={}", features.join(","));
    for arg in rest {
        out.push(' ');
        out.push_str(arg);
    }
    out
}

#[allow(unused_variables)]
fn attach_native_layer(app: &tauri::AppHandle, window: &tauri::WebviewWindow, backend: ViewerBackend) {
    if backend == ViewerBackend::Stream {
        return;
    }
    let below = backend == ViewerBackend::NativeBelow;
    let state = app.state::<AppState>();
    let layer_slot = state.layer.clone();
    let tx = relock(&state.viewer_tx).clone();

    let install = move |layer: Result<Box<dyn NativeLayer>, String>| match layer {
        Ok(layer) => {
            // SAFETY: the layer is kept alive in `layer_slot` for the life of the app.
            let target = viewer::SendTarget(unsafe { layer.surface_target() });
            let backend = layer.backend();
            *relock(&layer_slot) = Some(layer);
            let _ = tx.send(ViewerMsg::Attach { target, backend });
        }
        Err(e) => tracing::error!("native viewer layer unavailable, using the stream presenter: {e}"),
    };

    #[cfg(target_os = "macos")]
    {
        let _ = window.with_webview(move |wv| {
            // SAFETY: `inner()` is the live WKWebView; with_webview runs on the main thread.
            let layer = unsafe { viewer::macos::MetalLayerView::new(wv.inner(), below) };
            install(layer.map(|l| Box::new(l) as Box<dyn NativeLayer>));
        });
    }
    #[cfg(windows)]
    {
        match window.hwnd() {
            Ok(hwnd) => {
                let layer = viewer::windows::ChildSurfaceWindow::new(hwnd.0 as isize, below);
                install(layer.map(|l| Box::new(l) as Box<dyn NativeLayer>));
            }
            Err(e) => install(Err(e.to_string())),
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    install(Err("no native viewer layer on this platform".into()));
}

/// Builds and runs the app.
pub fn run() -> ExitCode {
    let started = Instant::now();
    let builder = specta_builder();
    let startup_paths =
        paths_from_args(&std::env::args().skip(1).collect::<Vec<_>>(), &std::env::current_dir().unwrap_or_default());

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let paths = paths_from_args(argv.get(1..).unwrap_or_default(), Path::new(&cwd));
            tracing::info!("second launch forwarded to the running instance");
            focus_main(app);
            route_paths(app, paths);
        }))
        .plugin(tauri_plugin_dialog::init())
        // Every command's panic is contained (see `worker`).
        .invoke_handler(worker::contain_handler(builder.invoke_handler()))
        .register_uri_scheme_protocol("ove-frame", |ctx, _request| {
            ctx.app_handle().state::<Arc<viewer::stream::StreamSink>>().response()
        })
        .setup(move |app| {
            builder.mount_events(app);
            let handle = app.handle().clone();
            let log_dir = app.path().app_log_dir().ok();
            let guard = log_dir.as_deref().map(logging::init);
            if let Some(g) = guard {
                app.manage(Mutex::new(g));
            }

            let (viewer_tx, viewer_rx) = std::sync::mpsc::channel();
            let stream = Arc::new(viewer::stream::StreamSink::default());
            let stats = Arc::new(Mutex::new(ViewerStats::default()));
            let seek_started = Arc::new(Mutex::new(None));
            let gpu_summary = Arc::new(Mutex::new(None));
            let (gpu_tx, gpu_rx) = std::sync::mpsc::channel();
            let frame_handle = handle.clone();
            let error_handle = handle.clone();
            let backend_handle = handle.clone();
            viewer::spawn(
                viewer_rx,
                stream.clone(),
                stats.clone(),
                seek_started.clone(),
                viewer::ViewerHooks {
                    on_stream_frame: Box::new(move |seq, width, height| {
                        let _ = ViewerFrameReady { seq: (seq % u64::from(u32::MAX)) as u32, width, height }
                            .emit(&frame_handle);
                    }),
                    on_error: Box::new(move |e| {
                        let _ = AppErrorRaised(e).emit(&error_handle);
                    }),
                    on_backend: Box::new(move |b| {
                        let _ = ViewerBackendChanged(b).emit(&backend_handle);
                    }),
                },
                gpu_tx,
            )?;
            {
                let gpu_summary = gpu_summary.clone();
                std::thread::Builder::new().name("startup-log".into()).spawn(move || {
                    let gpu = gpu_rx.recv().unwrap_or_else(|_| "unknown".into());
                    tracing::info!("{}", logging::startup_line(&gpu, DECODE_CAPABILITIES));
                    *relock(&gpu_summary) = Some(gpu);
                })?;
            }

            let engine = engine::spawn(handle.clone(), viewer_tx.clone())?;

            app.manage(stream);
            app.manage(AppState {
                engine,
                projects: projects::Projects::default(),
                assistant: assistant::Assistant::new(app.path().app_config_dir().ok()),
                viewer_tx: Mutex::new(viewer_tx),
                viewer_stats: stats,
                seek_started,
                layer: Arc::new(Mutex::new(None)),
                pending_paths: Mutex::new(startup_paths.clone()),
                ui_ready: AtomicBool::new(false),
                gpu_summary,
                log_dir,
            });

            // The command line / MCP server reach the app through the control link.
            control::start(&handle);
            app.manage(generation::Generation::default());

            create_main_window(app)?;
            if let Some(window) = app.get_webview_window("main") {
                restore_window(&handle, &window);
                attach_native_layer(&handle, &window, requested_backend());
                let _ = window.show();
            }
            tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "window shown");
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                route_paths(window.app_handle(), paths.iter().map(|p| p.display().to_string()).collect());
            }
            WindowEvent::CloseRequested { .. } => save_window(window.app_handle(), window),
            // Media may have come back (drive reconnected) or gone: check again, in the background.
            WindowEvent::Focused(true) => projects::refresh_media(window.app_handle()),
            WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Resized(_) => {
                // The UI re-measures and sends the viewer rectangle; nothing to do natively.
            }
            _ => {}
        })
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            tracing::error!(error = %e, "the app could not start");
            eprintln!("the app could not start: {e}");
            return ExitCode::FAILURE;
        }
    };

    app.run(|handle, event| {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let RunEvent::Opened { urls } = &event {
            let paths = urls.iter().filter_map(|u| u.to_file_path().ok()).map(|p| p.display().to_string()).collect();
            route_paths(handle, paths);
        }
        if let RunEvent::Exit = event {
            control::stop(handle);
            let state = handle.state::<AppState>();
            // Final snapshot, full flush, prune and lock release for the open project.
            if let Ok(mut open) = projects::lock(&state.projects.open)
                && let Some(p) = open.take()
                && let Err(e) = p.close()
            {
                tracing::error!(error = %e, "closing the project on exit failed; its revision log is intact");
            }
            state.send_viewer(ViewerMsg::Quit);
        }
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview2_args_keep_wry_defaults_and_merge_disabled_features() {
        let args = webview2_args(
            "--disable-features=IgnoreDuplicateNavs,Prewarm,msPdfOOUI --remote-debugging-port=0 --enable-automation",
        );
        assert_eq!(
            args,
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,IgnoreDuplicateNavs,Prewarm \
             --remote-debugging-port=0 --enable-automation"
        );
        assert_eq!(webview2_args(""), "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    }

    #[test]
    fn args_are_filtered_and_resolved() {
        let args = vec!["-v".to_string(), "psn_0_123".into(), "clip.mov".into(), "/abs/a.mp4".into()];
        let paths = paths_from_args(&args, Path::new("/work"));
        assert_eq!(paths, vec!["/work/clip.mov".to_string(), "/abs/a.mp4".to_string()]);
    }

    #[test]
    fn committed_bindings_are_current() {
        let committed = std::fs::read_to_string(bindings_path()).expect("ui/src/bindings.ts exists");
        check_bindings(&committed).unwrap();
    }

    mod contained {
        use crate::error::{AppError, ErrorCode};
        use crate::worker;

        #[tauri::command]
        pub fn boom() -> Result<(), AppError> {
            panic!("injected command panic")
        }

        #[tauri::command]
        pub async fn boom_async() -> Result<(), AppError> {
            worker::guarded("boom_async", async {
                if true {
                    panic!("injected async panic");
                }
                Ok(())
            })
            .await
        }

        #[tauri::command]
        pub fn fine() -> Result<u32, AppError> {
            Ok(7)
        }

        pub fn code(e: &serde_json::Value) -> ErrorCode {
            serde_json::from_value::<AppError>(e.clone()).expect("an AppError").code
        }
    }

    #[test]
    fn a_panicking_command_answers_with_an_error_and_the_app_keeps_working() {
        use tauri::ipc::{CallbackFn, InvokeBody};
        let app = tauri::test::mock_builder()
            .invoke_handler(worker::contain_handler(tauri::generate_handler![
                contained::boom,
                contained::boom_async,
                contained::fine
            ]))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let call = |cmd: &str| {
            tauri::test::get_ipc_response(
                &webview,
                tauri::webview::InvokeRequest {
                    cmd: cmd.into(),
                    callback: CallbackFn(0),
                    error: CallbackFn(1),
                    url: "tauri://localhost".parse().unwrap(),
                    body: InvokeBody::default(),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_string(),
                },
            )
        };
        for cmd in ["boom", "boom_async", "boom"] {
            let e = call(cmd).expect_err("a panic is an error answer");
            assert_eq!(contained::code(&e), crate::error::ErrorCode::WorkerPanic, "{cmd}: {e}");
            // The app still answers other commands.
            let ok = call("fine").expect("still working");
            assert_eq!(ok.deserialize::<u32>().unwrap(), 7);
        }
    }

    /// Async commands run on the async runtime, outside the invoke handler's panic containment, so each must run its
    /// body through `worker::blocking`, `worker::guarded` or `projects::serial`.
    #[test]
    fn every_async_command_contains_panics() {
        // Owned by other modules' maintainers; their async commands do their work in `spawn_blocking`.
        const EXEMPT: [&str; 2] = ["generation.rs", "interchange.rs"];
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut unguarded = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".rs") || EXEMPT.contains(&name.as_str()) {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap();
            for chunk in text.split("#[tauri::command]").skip(1) {
                let Some(sig) = chunk.lines().find(|l| l.starts_with("pub ")) else { continue };
                let body = chunk.split("\n}\n").next().unwrap_or_default();
                // `projects::import_paths` (shared with the assistant's import) runs through `worker::blocking` and
                // `serial` itself.
                if sig.contains("async fn")
                    && !["worker::blocking(", "worker::guarded(", "serial(", "import_paths("]
                        .iter()
                        .any(|g| body.contains(g))
                {
                    unguarded.push(format!("{name}: {sig}"));
                }
            }
        }
        assert!(unguarded.is_empty(), "async commands without panic containment: {unguarded:#?}");
    }

    #[test]
    fn stale_bindings_are_detected() {
        let committed = std::fs::read_to_string(bindings_path()).unwrap();
        let stale = committed.replace("seekFrame", "seekToFrame");
        assert_ne!(stale, committed, "fixture must change the contract");
        assert!(check_bindings(&stale).is_err());
    }
}
