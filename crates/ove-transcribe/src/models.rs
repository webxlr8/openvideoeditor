//! Speech models: whisper.cpp ggml files downloaded on first use into the app's data directory, verified by sha256.
//! Models are never shipped in the installer.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, TranscribeError};

/// Available models (multilingual). Sizes and hashes are those published in the whisper.cpp model repository.
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

    pub fn file_name(self) -> &'static str {
        match self {
            Model::Tiny => "ggml-tiny.bin",
            Model::Base => "ggml-base.bin",
            Model::Small => "ggml-small.bin",
        }
    }

    pub fn size_bytes(self) -> u64 {
        match self {
            Model::Tiny => 77_691_713,
            Model::Base => 147_951_465,
            Model::Small => 487_601_967,
        }
    }

    pub fn sha256(self) -> &'static str {
        match self {
            Model::Tiny => "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            Model::Base => "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
            Model::Small => "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        }
    }

    /// Short description for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Model::Tiny => "Fast — lower accuracy",
            Model::Base => "Balanced (recommended)",
            Model::Small => "Most accurate — slower",
        }
    }

    pub fn url(self) -> String {
        let base = std::env::var("OVE_WHISPER_MODEL_URL")
            .unwrap_or_else(|_| "https://huggingface.co/ggerganov/whisper.cpp/resolve/main".into());
        format!("{}/{}", base.trim_end_matches('/'), self.file_name())
    }
}

/// Where models live: `OVE_WHISPER_MODEL_DIR`, else `<data dir>/models`.
pub fn model_dir(data_dir: &Path) -> PathBuf {
    std::env::var_os("OVE_WHISPER_MODEL_DIR").map(PathBuf::from).unwrap_or_else(|| data_dir.join("models"))
}

pub fn model_path(dir: &Path, m: Model) -> PathBuf {
    dir.join(m.file_name())
}

/// True when the model file is present with the expected size (the hash was checked when it was downloaded).
pub fn is_installed(dir: &Path, m: Model) -> bool {
    std::fs::metadata(model_path(dir, m)).is_ok_and(|md| md.len() == m.size_bytes())
}

/// Downloads a model if it is missing. `progress(done, total)` is called as bytes arrive; `cancel` stops the
/// download (the partial file is removed). The file is verified by sha256 before it is moved into place.
pub fn ensure_model(dir: &Path, m: Model, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<PathBuf> {
    let path = model_path(dir, m);
    if is_installed(dir, m) {
        progress(m.size_bytes(), m.size_bytes());
        return Ok(path);
    }
    std::fs::create_dir_all(dir).map_err(|e| TranscribeError::Io(format!("create {}: {e}", dir.display())))?;
    let part = dir.join(format!("{}.part", m.file_name()));
    let result = (|| {
        let resp = ureq::get(&m.url())
            .timeout(std::time::Duration::from_secs(600))
            .call()
            .map_err(|e| TranscribeError::Download(format!("could not download the {} speech model: {e}", m.id())))?;
        let total = resp.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(m.size_bytes());
        let mut reader = resp.into_reader();
        let mut file =
            std::fs::File::create(&part).map_err(|e| TranscribeError::Io(format!("write {}: {e}", part.display())))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(TranscribeError::Cancelled);
            }
            let n =
                reader.read(&mut buf).map_err(|e| TranscribeError::Download(format!("download interrupted: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(|e| TranscribeError::Io(format!("write model: {e}")))?;
            done += n as u64;
            progress(done, total);
        }
        file.sync_all().map_err(|e| TranscribeError::Io(format!("write model: {e}")))?;
        let got = format!("{:x}", hasher.finalize());
        if got != m.sha256() {
            return Err(TranscribeError::Download(format!(
                "the downloaded {} model is damaged (checksum mismatch); try again",
                m.id()
            )));
        }
        std::fs::rename(&part, &path).map_err(|e| TranscribeError::Io(format!("install model: {e}")))?;
        Ok(path.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}
