//! Speech models: whisper.cpp ggml files, installed through the shared model store (`ove-models`) on first use and
//! verified by size and SHA-256. Models are never shipped in the installer.
//!
//! Files saved by earlier versions directly in `<data dir>/models` are adopted in place the first time they are
//! needed: hashed once and recorded in the store, so nothing downloads again.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use ove_models::manifest::{WHISPER, WhisperPin};
use ove_models::{ModelError, Store, UrlOverride};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TranscribeError};

/// Available models (multilingual).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    Tiny,
    Base,
    Small,
}

impl Model {
    pub const ALL: [Model; 3] = [Model::Tiny, Model::Base, Model::Small];

    pub fn id(self) -> &'static str {
        match self {
            Model::Tiny => "tiny",
            Model::Base => "base",
            Model::Small => "small",
        }
    }

    pub fn parse(s: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.id() == s)
    }

    /// The pinned file in the model store's catalog.
    fn pin(self) -> &'static WhisperPin {
        match self {
            Model::Tiny => &WHISPER[0],
            Model::Base => &WHISPER[1],
            Model::Small => &WHISPER[2],
        }
    }

    /// The model store id (`whisper-base`, …).
    pub fn store_id(self) -> &'static str {
        self.pin().id
    }

    pub fn file_name(self) -> &'static str {
        self.pin().file
    }

    pub fn size_bytes(self) -> u64 {
        self.pin().bytes
    }

    pub fn sha256(self) -> &'static str {
        self.pin().sha256
    }

    /// Short description for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Model::Tiny => "Fast — lower accuracy",
            Model::Base => "Balanced (recommended)",
            Model::Small => "Most accurate — slower",
        }
    }

    /// Where the file is fetched from: the pinned commit, `OVE_WHISPER_MODEL_URL/<file>`, or the store mirror.
    pub fn url(self) -> String {
        let m = self.pin().manifest();
        store_for(Path::new(".")).url_for(&m, &m.files[0])
    }
}

/// Where models live (the model store's root): `OVE_MODELS_DIR`, else `OVE_WHISPER_MODEL_DIR`, else
/// `<data dir>/models`.
pub fn model_dir(data_dir: &Path) -> PathBuf {
    ["OVE_MODELS_DIR", "OVE_WHISPER_MODEL_DIR"]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
        .unwrap_or_else(|| data_dir.join("models"))
}

/// The model store at `dir`. `OVE_WHISPER_MODEL_URL` (a folder of ggml files) still overrides the download location.
pub fn store_for(dir: &Path) -> Store {
    let flat = std::env::var("OVE_WHISPER_MODEL_URL").ok().filter(|s| !s.is_empty()).map(UrlOverride::Flat);
    let store = Store::open(dir);
    if flat.is_some() { store.with_url_override(flat) } else { store }
}

/// The model's file: where the store has it, else where it would be found from an earlier version.
pub fn model_path(dir: &Path, m: Model) -> PathBuf {
    store_for(dir).file_path(m.store_id(), m.file_name()).unwrap_or_else(|| legacy_path(dir, m))
}

/// Where earlier versions saved the file.
fn legacy_path(dir: &Path, m: Model) -> PathBuf {
    dir.join(m.file_name())
}

fn legacy_present(dir: &Path, m: Model) -> bool {
    std::fs::metadata(legacy_path(dir, m)).is_ok_and(|md| md.len() == m.size_bytes())
}

/// True when the model is installed in the store, or present from an earlier version with the expected size (its
/// hash is checked when it is first used).
pub fn is_installed(dir: &Path, m: Model) -> bool {
    store_for(dir).is_installed(m.store_id()) || legacy_present(dir, m)
}

/// Installs a model if it is missing. `progress(done, total)` is called as bytes arrive; `cancel` stops the
/// download (the partial file is removed). Files are verified by SHA-256 before they become visible.
pub fn ensure_model(dir: &Path, m: Model, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<PathBuf> {
    let store = store_for(dir);
    if !store.is_installed(m.store_id()) && legacy_present(dir, m) {
        store.adopt_in_place(m.store_id(), dir).map_err(|e| from_store(m, e))?;
    }
    let out = store.install(m.store_id(), &mut |p| progress(p.done, p.total), cancel).map_err(|e| from_store(m, e))?;
    out.paths.into_iter().next().ok_or_else(|| TranscribeError::Io("the speech model has no file".into()))
}

fn from_store(m: Model, e: ModelError) -> TranscribeError {
    match e {
        ModelError::Cancelled => TranscribeError::Cancelled,
        ModelError::Io(s) => TranscribeError::Io(s),
        ModelError::Corrupt { .. } => TranscribeError::Download(format!(
            "the downloaded {} model is damaged (checksum mismatch); try again",
            m.id()
        )),
        ModelError::Download(s) => {
            TranscribeError::Download(format!("could not download the {} speech model: {s}", m.id()))
        }
        other => TranscribeError::Download(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_match_the_store_catalog() {
        for m in Model::ALL {
            let man = ove_models::manifest::find(m.store_id()).unwrap();
            assert_eq!(man.files.len(), 1);
            assert_eq!(man.files[0].name, m.file_name());
            assert_eq!(man.files[0].bytes, m.size_bytes());
            assert_eq!(man.files[0].sha256, m.sha256());
        }
    }

    #[test]
    fn missing_model_is_not_installed_and_paths_fall_back_to_the_old_place() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_installed(dir.path(), Model::Tiny));
        assert_eq!(model_path(dir.path(), Model::Tiny), dir.path().join("ggml-tiny.bin"));
    }
}
