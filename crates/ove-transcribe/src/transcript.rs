//! Transcript types shared by recognition, the cache, captions and the timeline transcript. They need no
//! recognition library, so tools that only read cached transcripts build without it.

use serde::{Deserialize, Serialize};

/// Audio is recognised at this rate, mono.
pub const SAMPLE_RATE: u32 = 16_000;

/// One recognised word, times in microseconds from the start of the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
    /// Mean token probability (0–1).
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    /// ISO 639-1 code detected (or requested).
    pub language: String,
    pub model: String,
    pub words: Vec<Word>,
    pub duration_us: i64,
}

impl Transcript {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}
