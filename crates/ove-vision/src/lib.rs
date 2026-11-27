//! Footage indexing and search (roadmap change `add-semantic-media-search`).
//!
//! - [`tokenizer`] and [`preprocess`]: the image–text model's inputs.
//! - [`model`]: the model's encoders through the shared inference runtime (out of process in the model worker).
//! - [`index`]: one safetensors index file per media file, keyed by content identity, read through a memory map.
//! - `sampler`, `indexer` and `task` (feature `media`): shot-aware sampling and background indexing.
//! - [`rank`]: visual search over index files; [`spoken`]: ranked phrase search over transcripts.
//! - [`eval`]: recall@k and mean reciprocal rank for a labelled query set.

pub mod error;
pub mod eval;
pub mod index;
#[cfg(feature = "media")]
pub mod indexer;
pub mod model;
pub mod preprocess;
pub mod query;
pub mod rank;
#[cfg(feature = "media")]
pub mod sampler;
pub mod spoken;
#[cfg(feature = "media")]
pub mod task;
pub mod tokenizer;

pub use error::{Result, VisionError};

/// Version of the frame sampler. Index files record it; a change forces a rebuild.
pub const SAMPLER_VERSION: u32 = 1;
