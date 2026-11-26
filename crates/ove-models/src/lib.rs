//! Local machine-learning models (roadmap change `add-model-runtime`): manifests pinned by size and SHA-256 at
//! immutable locations, verified and resumable downloads with a free-space check, and the model store shared by every
//! feature and project (`<app data>/models`).

pub mod download;
pub mod error;
pub mod manifest;
pub mod space;
pub mod store;

pub use error::{ModelError, Result};
pub use manifest::{Manifest, ModelFile, RuntimeKind};
pub use store::{InstallOutcome, ModelStatus, Progress, Store, UrlOverride};

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

/// The model store's folder: `OVE_MODELS_DIR`, else `<app data>/models`.
pub fn default_root() -> Option<PathBuf> {
    std::env::var_os("OVE_MODELS_DIR").map(PathBuf::from).or_else(|| app_data_dir().map(|d| d.join("models")))
}
