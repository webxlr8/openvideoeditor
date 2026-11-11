//! The structured error every bridge command returns.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Machine-readable error category. The UI switches on this; `message` is shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    Unsupported,
    Corrupt,
    MediaOffline,
    NoMedia,
    InvalidArgument,
    WorkerPanic,
    Gpu,
    Io,
    Internal,
}

/// Error returned by every command: a code plus a plain-language message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        AppError { code, message: message.into() }
    }
}

impl From<ove_gpu::GpuError> for AppError {
    fn from(e: ove_gpu::GpuError) -> Self {
        AppError::new(ErrorCode::Gpu, format!("The viewer could not draw: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
