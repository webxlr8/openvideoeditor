use thiserror::Error;

/// Why a model operation failed. Messages are written for the person reading them in the app or the CLI.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ModelError {
    #[error("unknown model {0:?}")]
    UnknownModel(String),
    #[error("the manifest of {id} is not valid: {reason}")]
    InvalidManifest { id: String, reason: String },
    #[error(
        "not enough disk space for {model}: it needs {} MB and {} MB are free",
        mb_up(*needed),
        mb_down(*free)
    )]
    NoSpace { model: String, needed: u64, free: u64 },
    #[error("the download of {file} for {model} was corrupt (checksum mismatch); the file was deleted, try again")]
    Corrupt { model: String, file: String },
    #[error("{0}")]
    Download(String),
    #[error("{0}")]
    Io(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0} is already downloading")]
    Busy(String),
    /// The model worker failed or could not be reached.
    #[error("{0}")]
    Inference(String),
    /// The model ran but the job failed (bad inputs, an operator error).
    #[error("{0}")]
    JobFailed(String),
    /// The inference runtime library is not installed or could not be loaded.
    #[error("{0}")]
    RuntimeMissing(String),
}

impl ModelError {
    /// A stable code for JSON output and the app.
    pub fn code(&self) -> &'static str {
        match self {
            ModelError::UnknownModel(_) => "model.unknown",
            ModelError::InvalidManifest { .. } => "model.manifest",
            ModelError::NoSpace { .. } => "model.noSpace",
            ModelError::Corrupt { .. } => "model.corrupt",
            ModelError::Download(_) => "model.download",
            ModelError::Io(_) => "io",
            ModelError::Cancelled => "cancelled",
            ModelError::Busy(_) => "model.busy",
            ModelError::Inference(_) => "model.inference",
            ModelError::JobFailed(_) => "model.jobFailed",
            ModelError::RuntimeMissing(_) => "model.runtimeMissing",
        }
    }
}

/// Bytes as megabytes (10^6) with one decimal, rounded up (for amounts needed).
pub fn mb_up(bytes: u64) -> String {
    let tenths = bytes.div_ceil(100_000);
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// A download or model size as people read it: gigabytes (10^9) from 500 MB up (two decimals below 1 GB, so the
/// visual search model reads "0.63 GB"), otherwise megabytes with one decimal, rounded up.
pub fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 500_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", mb_up(bytes))
    }
}

/// Bytes as megabytes (10^6) with one decimal, rounded down (for amounts available).
pub fn mb_down(bytes: u64) -> String {
    let tenths = bytes / 100_000;
    format!("{}.{}", tenths / 10, tenths % 10)
}

pub type Result<T> = std::result::Result<T, ModelError>;
