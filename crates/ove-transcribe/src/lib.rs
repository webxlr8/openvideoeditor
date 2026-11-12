//! Offline speech-to-text for captions (roadmap change 10): whisper.cpp models downloaded on first use, word-level
//! timestamps, caption segmentation, and a per-media transcript cache in the project bundle.

pub mod cache;
pub mod error;
pub mod models;
pub mod recognize;
pub mod segment;

pub use error::{Result, TranscribeError};
pub use models::Model;
pub use recognize::{
    AudioWindow, AudioWindows, OVERLAP_SECONDS, Options, Transcript, WINDOW_SECONDS, Word, audio_windows, load_audio,
    transcribe, transcribe_file,
};
pub use segment::{Segment, SegmentOptions, segment};
