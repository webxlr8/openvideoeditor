//! Media and timeline analysis for the CLI and agents: frames and contact sheets of media files, scene cuts, audio
//! levels, waveform pictures, silence, loudness (BS.1770 / EBU R128), and composited timeline frames with exposure
//! statistics. Results are plain serializable structs or RGBA images (`Image::png`); nothing is printed.
#![forbid(unsafe_code)]

pub mod audio;
pub mod image;
pub mod look;
pub mod video;

pub use audio::{Levels, Loudness, Range, levels, loudness, silence, waveform_png};
pub use image::{Image, Stats, frame_stats, sheet};
pub use look::{look, timeline_times};
pub use video::{FilmstripOpts, SceneCut, Scenes, filmstrip, frame_to_image, grab, scenes};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Media(#[from] ove_media::MediaError),
    #[error("no frame at {at} s in {path}")]
    NoFrame { path: String, at: f64 },
    #[error("{0} has no audio")]
    NoAudio(String),
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    /// Stable code for JSON errors.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Media(_) => "media.unreadable",
            Error::NoFrame { .. } => "media.noFrame",
            Error::NoAudio(_) => "media.noAudio",
            Error::Invalid(_) => "invalid.argument",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
