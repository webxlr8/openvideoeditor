//! Rendering the timeline: evaluation (what is visible when), a CPU compositor, text and captions, audio mixdown,
//! and export to MP4 (H.264 + AAC). Preview and export share this code, so what plays is what exports.

pub mod audio;
pub mod backend;
pub mod canvas;
pub mod captions;
pub mod captions_presets;
pub mod color;
pub mod effects;
pub mod eval;
pub mod export;
pub mod fonts;
pub mod geometry;
pub mod layouts;
pub mod loudness;
pub mod meter;
pub mod parity;
pub mod proxy;
pub mod queue;
pub mod render;
pub mod scopes;
pub mod sources;
pub mod text;
pub mod text_anim;
pub mod transitions;

pub use backend::{Backend, CpuBackend, Tex};
pub use canvas::Rgba;
pub use export::{ExportError, ExportResult, ExportSettings, Progress, export};
pub use render::Renderer;
