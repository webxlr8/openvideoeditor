//! Store errors with stable codes and plain-language messages.

use std::io;
use std::path::Path;

use ove_core::BatchError;
use ove_core::model::ReadError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Batch(#[from] BatchError),
    #[error("{path}: {source}")]
    Invalid { path: String, source: ReadError },
    #[error("project is open in {holder}")]
    Locked { holder: String },
    #[error("{message}")]
    Io { code: &'static str, message: String, source: io::Error },
    #[error("{message}")]
    Refused { code: &'static str, message: String },
}

impl StoreError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::Batch(e) => e.code,
            StoreError::Invalid { source, .. } => source.code,
            StoreError::Locked { .. } => "project.locked",
            StoreError::Io { code, .. } => code,
            StoreError::Refused { code, .. } => code,
        }
    }

    pub fn io(what: &str, path: &Path, e: io::Error) -> Self {
        let (code, reason) = match e.kind() {
            io::ErrorKind::StorageFull => ("disk.full", "the disk is full".to_string()),
            io::ErrorKind::QuotaExceeded => ("disk.full", "the disk quota is exceeded".to_string()),
            io::ErrorKind::PermissionDenied => ("permission.denied", "permission was denied".to_string()),
            io::ErrorKind::ResourceBusy => ("file.busy", "the file is in use by another process".to_string()),
            io::ErrorKind::NotFound => ("not.found", "it does not exist".to_string()),
            io::ErrorKind::ReadOnlyFilesystem => ("permission.denied", "the volume is read-only".to_string()),
            _ => ("io", e.to_string()),
        };
        StoreError::Io { code, message: format!("could not {what} {}: {reason}", path.display()), source: e }
    }

    pub fn refused(code: &'static str, message: impl Into<String>) -> Self {
        StoreError::Refused { code, message: message.into() }
    }
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;
