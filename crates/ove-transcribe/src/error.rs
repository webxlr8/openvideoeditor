use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TranscribeError {
    #[error("{0}")]
    Download(String),
    #[error("{0}")]
    Io(String),
    #[error("could not read the audio: {0}")]
    Audio(String),
    #[error("speech recognition failed: {0}")]
    Model(String),
    #[error("cancelled")]
    Cancelled,
    #[error("the file has no audio track")]
    NoAudio,
}

impl TranscribeError {
    pub fn code(&self) -> &'static str {
        match self {
            TranscribeError::Download(_) => "transcribe.download",
            TranscribeError::Io(_) => "io",
            TranscribeError::Audio(_) => "transcribe.audio",
            TranscribeError::Model(_) => "transcribe.model",
            TranscribeError::Cancelled => "cancelled",
            TranscribeError::NoAudio => "transcribe.noAudio",
        }
    }
}

pub type Result<T> = std::result::Result<T, TranscribeError>;
