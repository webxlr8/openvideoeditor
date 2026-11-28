//! The compositing backend of the app (design D9): one choice for timeline preview, colour tools and export. The
//! GPU is used when the person allows it (Settings → Rendering, on by default), a hardware adapter is found and the
//! startup self-test matches the CPU reference; otherwise the CPU. The reason is shown in the diagnostics overlay and
//! in Settings.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ove_gpu::compositor::{Choice, select};
use ove_render::{Backend, CpuBackend, Renderer};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::AppResult;
use crate::worker::relock;

const FILE: &str = "compositing.json";

/// The stored preference.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    gpu: bool,
}

/// What Settings and the diagnostics overlay show.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompositingStatus {
    /// The person allows the GPU (Settings → Rendering).
    pub gpu_enabled: bool,
    /// The backend in use: "gpu" or "cpu"; "starting" until the startup self-test has run.
    pub backend: String,
    pub adapter: Option<String>,
    /// Why this backend.
    pub reason: String,
}

struct State {
    file: Option<PathBuf>,
    gpu_enabled: bool,
    status: Option<CompositingStatus>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    // Unit tests of the app do not depend on the machine's GPU (ove-gpu tests the GPU backend); the app itself
    // starts with the GPU allowed and reads the setting in [`init`].
    S.get_or_init(|| Mutex::new(State { file: None, gpu_enabled: !cfg!(test), status: None }))
}

fn choice(gpu_enabled: bool) -> Choice {
    if gpu_enabled { Choice::from_env_or(Choice::Auto) } else { Choice::Cpu }
}

/// A backend for a new renderer, and the status it gives.
pub fn backend() -> (Box<dyn Backend>, CompositingStatus) {
    let enabled = relock(state()).gpu_enabled;
    let (backend, status): (Box<dyn Backend>, CompositingStatus) = match select(choice(enabled)) {
        Ok(s) => {
            let status = CompositingStatus {
                gpu_enabled: enabled,
                backend: s.name.into(),
                adapter: s.adapter.clone(),
                reason: if enabled { s.reason } else { "the GPU is turned off in Settings".into() },
            };
            (s.backend, status)
        }
        Err(e) => (
            Box::new(CpuBackend::new()),
            CompositingStatus { gpu_enabled: enabled, backend: "cpu".into(), adapter: None, reason: e },
        ),
    };
    relock(state()).status = Some(status.clone());
    (backend, status)
}

/// A renderer on the chosen backend.
pub fn renderer(bundle: Option<&Path>, height: u32) -> (Renderer, CompositingStatus) {
    let (b, status) = backend();
    (Renderer::with_backend(bundle, height, b), status)
}

/// Records that a renderer left the GPU (device lost).
pub fn fell_back(reason: &str) {
    let mut s = relock(state());
    let enabled = s.gpu_enabled;
    s.status =
        Some(CompositingStatus { gpu_enabled: enabled, backend: "cpu".into(), adapter: None, reason: reason.into() });
}

/// One line for the diagnostics overlay: "GPU (adapter)" or "CPU — reason".
pub fn label(s: &CompositingStatus) -> String {
    match (s.backend.as_str(), &s.adapter) {
        ("gpu", Some(a)) => format!("GPU, {a}"),
        ("gpu", None) => "GPU".into(),
        ("starting", _) => "starting".into(),
        _ => format!("CPU — {}", s.reason),
    }
}

/// Reads the stored preference and runs the GPU self-test once in the background, so the first preview does not
/// wait for it.
pub fn init(app: &AppHandle) {
    let file = app.path().app_config_dir().ok().map(|d| d.join(FILE));
    let enabled = file
        .as_ref()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
        .is_none_or(|s| s.gpu);
    {
        let mut s = relock(state());
        s.file = file;
        s.gpu_enabled = enabled;
    }
    // Queued exports (and any render without its own backend) use the same choice.
    ove_render::backend::set_provider(|choice| match choice.and_then(Choice::parse) {
        Some(c) => select(c).map(|s| (s.backend, format!("{}: {}", s.name, s.reason))),
        None => {
            let (b, s) = backend();
            Ok((b, format!("{}: {}", s.backend, s.reason)))
        }
    });
    let _ = crate::worker::spawn_contained("gpu self-test", || {
        let (_, status) = backend();
        tracing::info!(backend = %status.backend, reason = %status.reason, "compositing backend");
    });
}

/// The current compositing status.
#[tauri::command]
#[specta::specta]
pub fn compositing_status() -> AppResult<CompositingStatus> {
    let s = relock(state());
    Ok(s.status.clone().unwrap_or(CompositingStatus {
        gpu_enabled: s.gpu_enabled,
        backend: "starting".into(),
        adapter: None,
        reason: "the GPU self-test has not finished".into(),
    }))
}

/// Allows or forbids the GPU for compositing. Applies to timeline previews built (the timeline is rebuilt on every
/// edit and on open) and exports started after the change.
#[tauri::command]
#[specta::specta]
pub async fn set_gpu_compositing(enabled: bool) -> AppResult<CompositingStatus> {
    crate::worker::blocking("set_gpu_compositing", move || {
        let file = {
            let mut s = relock(state());
            s.gpu_enabled = enabled;
            s.file.clone()
        };
        if let Some(f) = file {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(&f, serde_json::to_vec(&Stored { gpu: enabled }).unwrap_or_default()) {
                tracing::warn!("could not save the compositing setting: {e}");
            }
        }
        let (_, status) = backend();
        Ok(status)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(backend: &str, adapter: Option<&str>, reason: &str) -> CompositingStatus {
        CompositingStatus {
            gpu_enabled: true,
            backend: backend.into(),
            adapter: adapter.map(Into::into),
            reason: reason.into(),
        }
    }

    #[test]
    fn the_overlay_names_the_backend_and_the_reason() {
        assert_eq!(label(&status("gpu", Some("Mock GPU"), "GPU self-test passed")), "GPU, Mock GPU");
        let failed =
            status("cpu", None, "GPU self-test failed on Mock GPU (blend hue: max 9/255): using the CPU backend");
        assert!(label(&failed).starts_with("CPU — GPU self-test failed"), "{}", label(&failed));
        assert_eq!(choice(false), Choice::Cpu, "the setting forces the CPU");
    }
}
