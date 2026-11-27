//! Footage indexing and search (roadmap change `add-semantic-media-search`).
//!
//! - [`tokenizer`] and [`preprocess`]: the image–text model's inputs.
//! - [`model`]: the model's encoders through the shared inference runtime (out of process in the model worker).
//! - [`index`]: one safetensors index file per media file, keyed by content identity, read through a memory map.

pub mod error;
pub mod index;
pub mod model;
pub mod preprocess;
pub mod tokenizer;

pub use error::{Result, VisionError};

/// Version of the frame sampler. Index files record it; a change forces a rebuild.
pub const SAMPLER_VERSION: u32 = 1;
