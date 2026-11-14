//! Rendering the timeline: evaluation (what is visible when), a CPU compositor, text and captions, audio mixdown,
//! and export to MP4 (H.264 + AAC). Preview and export share this code, so what plays is what exports.

pub mod audio;
pub mod canvas;
pub mod captions;
pub mod captions_presets;
pub mod color;
pub mod effects;
pub mod eval;
pub mod export;
pub mod fonts;
pub mod layouts;
pub mod render;
pub mod sources;
pub mod text;
pub mod text_anim;
pub mod transitions;

pub use canvas::Rgba;
pub use export::{ExportError, ExportResult, ExportSettings, Progress, export};
pub use render::Renderer;
