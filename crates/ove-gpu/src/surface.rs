//! Swap-chain surface for the native viewer.

use crate::renderer::{GpuError, RenderInfo, ViewerRenderer};

/// Result of presenting one frame to a surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented(RenderInfo),
    /// Window occluded / minimised, or a timeout: nothing drawn, try next frame.
    Skipped,
    /// The surface was outdated or lost and has been reconfigured; draw again.
    Reconfigured,
    /// The device was lost and recreated (frame re-uploaded); draw again.
    Recovered,
}

/// A configured swap chain sized to the viewer rectangle in physical pixels.
pub struct ViewerSurface {
    surface: wgpu::Surface<'static>,
    config: Option<wgpu::SurfaceConfiguration>,
}

impl std::fmt::Debug for ViewerSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewerSurface")
            .field("size", &self.size())
            .finish()
    }
}

impl ViewerSurface {
    /// Wraps a surface created from the renderer's instance (see [`ViewerRenderer::context`]).
    pub fn new(surface: wgpu::Surface<'static>) -> Self {
        ViewerSurface {
            surface,
            config: None,
        }
    }

    /// Creates a surface from a raw platform target (CAMetalLayer, HWND, DComp visual, …).
    ///
    /// # Safety
    /// The target must stay valid until this surface is dropped.
    pub unsafe fn from_raw(
        renderer: &ViewerRenderer,
        target: wgpu::SurfaceTargetUnsafe,
    ) -> Result<Self, GpuError> {
        // SAFETY: forwarded to the caller.
        let surface = unsafe { renderer.context().instance.create_surface_unsafe(target) }
            .map_err(|e| GpuError::Surface(e.to_string()))?;
        Ok(ViewerSurface::new(surface))
    }

    pub fn surface(&self) -> &wgpu::Surface<'static> {
        &self.surface
    }

    pub fn size(&self) -> (u32, u32) {
        self.config
            .as_ref()
            .map(|c| (c.width, c.height))
            .unwrap_or((0, 0))
    }

    /// (Re)configures for a physical size; zero sizes leave the surface unconfigured (hidden).
    pub fn resize(&mut self, renderer: &ViewerRenderer, width: u32, height: u32) {
        if width == 0 || height == 0 {
            self.config = None;
            return;
        }
        let device = &renderer.context().device;
        let caps = self.surface.get_capabilities(&renderer.context().adapter);
        // Video values are already gamma encoded: prefer a non-sRGB format so they are written as-is.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        self.surface.configure(device, &config);
        self.config = Some(config);
    }

    /// Draws the renderer's current frame and presents it.
    pub fn present(&mut self, renderer: &mut ViewerRenderer) -> Result<PresentOutcome, GpuError> {
        if renderer.recover_if_lost(Some(&self.surface))? {
            if let Some((w, h)) = self.config.as_ref().map(|c| (c.width, c.height)) {
                self.resize(renderer, w, h);
            }
            return Ok(PresentOutcome::Recovered);
        }
        let Some(config) = self.config.clone() else {
            return Ok(PresentOutcome::Skipped);
        };
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(PresentOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.resize(renderer, config.width, config.height);
                return Ok(PresentOutcome::Reconfigured);
            }
            other => return Err(GpuError::Surface(format!("{other:?}"))),
        };
        let view = texture.texture.create_view(&Default::default());
        let info = renderer.render(&view, config.format, config.width, config.height);
        renderer.context().queue.present(texture);
        Ok(PresentOutcome::Presented(info))
    }
}
