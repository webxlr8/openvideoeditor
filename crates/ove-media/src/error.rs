use std::path::{Path, PathBuf};

use serde::Serialize;

/// Every failure leaving this crate. Each variant has a stable `code()` and a plain-language message; nothing in
/// this crate panics across its public API (worker threads catch panics and report [`MediaError::WorkerPanicked`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MediaError {
    #[error("{path}: the file does not exist")]
    NotFound { path: PathBuf },
    #[error("{path}: media offline ({reason})")]
    Offline { path: PathBuf, reason: String },
    #[error("{path}: this file can't be played ({reason})")]
    Unsupported { path: PathBuf, reason: String },
    #[error("{path}: the file is damaged or incomplete ({reason})")]
    Corrupt { path: PathBuf, reason: String },
    #[error("decoding failed: {0}")]
    Decode(String),
    #[error("hardware decoder failed: {0}")]
    Hardware(String),
    #[error("the decoder stopped unexpectedly: {0}")]
    WorkerPanicked(String),
    #[error("no {0} stream in this file")]
    NoStream(&'static str),
}

impl MediaError {
    /// Stable machine-readable code (`media.<kind>`), used by the UI bridge and logs.
    pub fn code(&self) -> &'static str {
        match self {
            MediaError::NotFound { .. } => "media.not_found",
            MediaError::Offline { .. } => "media.offline",
            MediaError::Unsupported { .. } => "media.unsupported",
            MediaError::Corrupt { .. } => "media.corrupt",
            MediaError::Decode(_) => "media.decode",
            MediaError::Hardware(_) => "media.hardware",
            MediaError::WorkerPanicked(_) => "media.worker_panicked",
            MediaError::NoStream(_) => "media.no_stream",
        }
    }

    pub(crate) fn from_open(path: &Path, err: ffmpeg_the_third::Error) -> MediaError {
        use ffmpeg_the_third::Error as E;
        if !path.exists() {
            return MediaError::NotFound { path: path.to_owned() };
        }
        let path = path.to_owned();
        match err {
            E::InvalidData | E::Eof => MediaError::Corrupt { path, reason: err.to_string() },
            E::DecoderNotFound | E::DemuxerNotFound | E::PatchWelcome | E::Bug => {
                MediaError::Unsupported { path, reason: err.to_string() }
            }
            E::Other { errno } if errno == libc::EACCES => {
                MediaError::Offline { path, reason: "permission denied".into() }
            }
            other => MediaError::Unsupported { path, reason: other.to_string() },
        }
    }
}

pub type Result<T> = std::result::Result<T, MediaError>;
