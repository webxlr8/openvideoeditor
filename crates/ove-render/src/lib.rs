//! Rendering the timeline: evaluation (what is visible when), a CPU compositor, text and captions, audio mixdown,
//! and export to MP4 (H.264 + AAC). Preview and export share this code, so what plays is what exports.

pub mod audio;
pub mod canvas;
pub mod eval;
pub mod export;
pub mod render;
pub mod sources;
pub mod text;

pub use canvas::Rgba;
pub use export::{ExportError, ExportResult, ExportSettings, Progress, export};
pub use render::Renderer;
