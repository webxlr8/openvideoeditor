//! Offline speech-to-text for captions (roadmap change 10): whisper.cpp models downloaded on first use, word-level
//! timestamps, caption segmentation, and a per-media transcript cache in the project bundle. The timeline transcript,
//! filler lists, the cache and voice activity through the shared runtime (add-transcript-editing) build without the
//! recognition library: turn off the default `recognize` feature for tools that only read transcripts.

pub mod cache;
pub mod cuts;
pub mod error;
pub mod fillers;
pub mod levels;
pub mod models;
#[cfg(feature = "recognize")]
pub mod recognize;
pub mod segment;
pub mod timeline;
pub mod transcript;
pub mod vad;

pub use error::{Result, TranscribeError};
pub use models::Model;
#[cfg(feature = "recognize")]
pub use recognize::{
    AudioWindow, AudioWindows, OVERLAP_SECONDS, Options, WINDOW_SECONDS, audio_windows, load_audio, transcribe,
    transcribe_file,
};
pub use segment::{Segment, SegmentOptions, segment};
pub use transcript::{SAMPLE_RATE, Transcript, Word};
