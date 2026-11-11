//! Zero-copy import of hardware-decoded frames (design D4).
//!
//! The decoder hands over a platform handle instead of CPU planes; the renderer wraps it as a
//! texture without copying. These hooks define the interface. The platform bodies are not
//! implemented yet: they return [`ImportError::Unsupported`] and callers fall back to the
//! one-copy upload path ([`crate::ViewerRenderer::upload`]), which the diagnostics report as
//! `copies_last_frame = 1`.
//!
//! UNVERIFIED: nothing in this module has run on macOS or Windows yet.

use crate::renderer::ViewerRenderer;

/// A hardware frame owned by the decoder.
#[derive(Debug, Clone, Copy)]
pub enum NativeFrameHandle {
    /// `IOSurfaceRef` of a VideoToolbox `CVPixelBuffer` (NV12 / P010 bi-planar).
    #[cfg(target_os = "macos")]
    IoSurface(*mut std::ffi::c_void),
    /// NT shared handle of a D3D11 texture (D3D11VA output), opened on D3D12 with
    /// `OpenSharedHandle`.
    #[cfg(windows)]
    D3d11Shared(isize),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("zero-copy import is not available on this platform/driver; use the upload path")]
    Unsupported,
}

/// Attempts to show a hardware frame without a CPU copy.
pub fn try_import(
    _renderer: &mut ViewerRenderer,
    _handle: NativeFrameHandle,
) -> Result<(), ImportError> {
    // macOS plan: CVMetalTextureCacheCreateTextureFromImage per plane → wgpu-hal Metal texture →
    // `Device::create_texture_from_hal`, then the same conversion pass as the upload path.
    // Windows plan: ID3D12Device::OpenSharedHandle → wgpu-hal DX12 texture → create_texture_from_hal.
    Err(ImportError::Unsupported)
}
