//! GPU side of the viewer: device management, frame upload (NV12 / P010 / RGBA8), YUV→RGB
//! conversion, letterboxed presentation to a surface or offscreen target, and device-loss
//! recovery. No UI or decoder dependency: the decoder produces [`VideoFrame`]s, the shell
//! provides a surface target.

pub mod cpu;
pub mod fit;
pub mod frame;
pub mod import;
mod renderer;
mod shaders;
pub mod surface;

pub use fit::{LogicalRect, PhysicalRect, fit_rect};
pub use frame::{FrameError, PixelFormat, VideoFrame, YuvMatrix};
pub use renderer::{
    AdapterSummary, GpuContext, GpuError, GpuOptions, RenderInfo, UploadStats, ViewerRenderer,
};
pub use surface::{PresentOutcome, ViewerSurface};
pub use wgpu;
