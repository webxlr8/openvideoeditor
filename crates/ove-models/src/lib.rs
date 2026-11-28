//! Local machine-learning models (roadmap change `add-model-runtime`): manifests pinned by size and SHA-256 at
//! immutable locations, verified and resumable downloads with a free-space check, and the model store shared by every
//! feature and project (`<app data>/models`).

pub mod archive;
pub mod check;
pub mod download;
pub mod error;
pub mod infer;
pub mod manifest;
#[cfg(feature = "onnx")]
pub mod onnx;
pub mod runtime;
pub mod space;
pub mod store;
#[cfg(feature = "tract")]
pub mod tract;
pub mod worker;

pub use error::{ModelError, Result};
pub use manifest::{ArchiveMember, Manifest, ModelFile, RuntimeKind};
pub use store::{
    CONSENT_ABOVE_BYTES, Ensure, InstallOutcome, Lease, ModelStatus, NotInstalled, Origin, Progress, Store,
    UrlOverride, needs_consent,
};

use std::path::PathBuf;

/// The app's data folder, the same one the desktop app uses: `%APPDATA%\dev.openvideoeditor.app` on Windows,
/// `~/Library/Application Support/dev.openvideoeditor.app` on macOS, `$XDG_DATA_HOME` (or `~/.local/share`) plus the
/// identifier elsewhere.
pub fn app_data_dir() -> Option<PathBuf> {
    const IDENTIFIER: &str = "dev.openvideoeditor.app";
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
    };
    base.map(|b| b.join(IDENTIFIER))
}

/// The model store's folder: `OVE_MODELS_DIR`, else `OVE_WHISPER_MODEL_DIR` (the former speech-model setting), else
/// `<app data>/models`. The app and the CLI resolve it the same way.
pub fn default_root() -> Option<PathBuf> {
    ["OVE_MODELS_DIR", "OVE_WHISPER_MODEL_DIR"]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
        .or_else(|| app_data_dir().map(|d| d.join("models")))
}
