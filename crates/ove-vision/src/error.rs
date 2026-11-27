//! Errors of footage indexing and search.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VisionError {
    /// The visual model (or the inference runtime) is not installed. Nothing is downloaded.
    #[error("model not installed: {message}")]
    ModelNotInstalled { message: String, bytes: u64 },
    #[error("tokenizer: {0}")]
    Tokenizer(String),
    #[error("the model failed: {0}")]
    Model(String),
    /// An index file is damaged or from another model or sampler; it is discarded and rebuilt.
    #[error("index {path}: {reason}")]
    Index { path: String, reason: String },
    #[error("{0}")]
    Media(String),
    #[error("cancelled")]
    Cancelled,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl VisionError {
    /// Stable error code for tools and the command line.
    pub fn code(&self) -> &'static str {
        match self {
            VisionError::ModelNotInstalled { .. } => "model.notInstalled",
            VisionError::Tokenizer(_) => "model.tokenizer",
            VisionError::Model(_) => "model.failed",
            VisionError::Index { .. } => "index.damaged",
            VisionError::Media(_) => "media.unreadable",
            VisionError::Cancelled => "cancelled",
            VisionError::Io(_) => "io",
        }
    }
}

pub type Result<T> = std::result::Result<T, VisionError>;
