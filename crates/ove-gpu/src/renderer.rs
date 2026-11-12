//! GPU device management and the viewer renderer.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::cpu::{range_params, tone_params, yuv_matrix};
use crate::fit::{PhysicalRect, fit_rect};
use crate::frame::{FrameError, PixelFormat, Transfer, VideoFrame};
use crate::shaders;

/// Errors from the GPU layer. All are recoverable by the caller (show a message, retry, or fall
/// back to the CPU presenter); none panic.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("no GPU adapter is available: {0}")]
    NoAdapter(String),
    #[error("could not open the GPU device: {0}")]
    Device(String),
    #[error("could not create the viewer surface: {0}")]
    Surface(String),
    #[error("the GPU device was lost: {0}")]
    DeviceLost(String),
    #[error("invalid frame: {0}")]
    Frame(#[from] FrameError),
    #[error("GPU readback failed: {0}")]
    Readback(String),
}

/// Adapter description for logs and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterSummary {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}

impl std::fmt::Display for AdapterSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}, {}, driver {})", self.name, self.backend, self.device_type, self.driver)
    }
}

/// Options for opening the GPU.
#[derive(Debug, Clone, Default)]
pub struct GpuOptions {
    /// Allow a software adapter (lavapipe / WARP). Used by headless tests.
    pub allow_software: bool,
}

fn backends() -> wgpu::Backends {
    if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else if cfg!(windows) {
        wgpu::Backends::DX12
    } else {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    }
}

/// Instance + adapter + device + queue, with a device-lost flag.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    lost: Arc<AtomicBool>,
    lost_reason: Arc<std::sync::Mutex<String>>,
    options: GpuOptions,
    /// Increments every time the device is recreated.
    pub generation: u64,
}

impl GpuContext {
    /// Opens the GPU, preferring a hardware adapter.
    pub fn new(options: GpuOptions) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        Self::with_instance(instance, None, options, 0)
    }

    /// Opens the GPU on an existing instance, compatible with `surface` when given.
    pub fn with_instance(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        options: GpuOptions,
        generation: u64,
    ) -> Result<Self, GpuError> {
        let request = |force_fallback_adapter| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter,
                compatible_surface: surface,
                ..Default::default()
            }))
        };
        let adapter = match request(false) {
            Ok(a) => a,
            Err(e) if options.allow_software => {
                request(true).map_err(|e2| GpuError::NoAdapter(format!("{e}; {e2}")))?
            }
            Err(e) => return Err(GpuError::NoAdapter(e.to_string())),
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ove-viewer"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;

        let lost = Arc::new(AtomicBool::new(false));
        let lost_reason = Arc::new(std::sync::Mutex::new(String::new()));
        {
            let lost = lost.clone();
            let lost_reason = lost_reason.clone();
            device.set_device_lost_callback(move |reason, message| {
                tracing::warn!(?reason, %message, "GPU device lost");
                if let Ok(mut r) = lost_reason.lock() {
                    *r = format!("{reason:?}: {message}");
                }
                lost.store(true, Ordering::SeqCst);
            });
        }
        device.on_uncaptured_error(Arc::new(|e| tracing::error!(error = %e, "uncaptured GPU error")));
        Ok(GpuContext { instance, adapter, device, queue, lost, lost_reason, options, generation })
    }

    pub fn summary(&self) -> AdapterSummary {
        let info = self.adapter.get_info();
        AdapterSummary {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            driver: if info.driver_info.is_empty() { info.driver } else { info.driver_info },
        }
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    pub fn lost_reason(&self) -> String {
        self.lost_reason.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Destroys the device as a driver reset would. Test and diagnostics hook.
    pub fn simulate_device_loss(&self) {
        self.device.destroy();
        // Some backends report the loss lazily; make the state observable immediately.
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.lost.store(true, Ordering::SeqCst);
    }

    /// Recreates adapter and device on the same instance (so existing surfaces stay valid).
    pub fn recreate(&self, surface: Option<&wgpu::Surface<'_>>) -> Result<GpuContext, GpuError> {
        GpuContext::with_instance(self.instance.clone(), surface, self.options.clone(), self.generation + 1)
    }
}

/// Upload counters for the diagnostics overlay.
#[derive(Debug, Default)]
pub struct UploadStats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    /// CPU→GPU copies of the most recent frame (1 for the upload path, 0 for zero-copy import).
    pub copies_last_frame: AtomicU64,
}

struct SourceTextures {
    format: PixelFormat,
    width: u32,
    height: u32,
    planes: Vec<wgpu::Texture>,
    intermediate: wgpu::Texture,
    convert_bind: wgpu::BindGroup,
    present_bind: wgpu::BindGroup,
}

struct Pipelines {
    convert_layouts: HashMap<PixelFormat, wgpu::BindGroupLayout>,
    convert: HashMap<PixelFormat, wgpu::RenderPipeline>,
    present_layout: wgpu::BindGroupLayout,
    present: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
}

const INTERMEDIATE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Uploads decoded frames and draws the current one letterboxed into a target.
pub struct ViewerRenderer {
    ctx: GpuContext,
    pipelines: Pipelines,
    source: Option<SourceTextures>,
    current: Option<VideoFrame>,
    current_transfer: Transfer,
    /// Background colour around the image (linear 0..1 RGBA, written as-is).
    pub background: wgpu::Color,
    pub stats: UploadStats,
}

impl std::fmt::Debug for ViewerRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewerRenderer").field("generation", &self.ctx.generation).finish()
    }
}

fn plane_texture_format(format: PixelFormat, plane: usize) -> wgpu::TextureFormat {
    match (format, plane) {
        (PixelFormat::Rgba8, _) => wgpu::TextureFormat::Rgba8Unorm,
        (PixelFormat::Nv12, 0) => wgpu::TextureFormat::R8Unorm,
        (PixelFormat::Nv12, _) => wgpu::TextureFormat::Rg8Unorm,
        (PixelFormat::P010, 0) => wgpu::TextureFormat::R16Uint,
        (PixelFormat::P010, _) => wgpu::TextureFormat::Rg16Uint,
    }
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let mut convert_layouts = HashMap::new();
        let mut convert = HashMap::new();
        for format in [PixelFormat::Rgba8, PixelFormat::Nv12, PixelFormat::P010] {
            let sample_type = if format == PixelFormat::P010 {
                wgpu::TextureSampleType::Uint
            } else {
                wgpu::TextureSampleType::Float { filterable: false }
            };
            let mut entries = vec![wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }];
            for plane in 0..format.plane_count() {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding: plane as u32 + 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
            }
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("convert"),
                entries: &entries,
            });
            let source = match format {
                PixelFormat::Rgba8 => shaders::rgba_convert(),
                PixelFormat::Nv12 => shaders::nv12_convert(),
                PixelFormat::P010 => shaders::p010_convert(),
            };
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("convert"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = render_pipeline(device, &layout, &module, INTERMEDIATE_FORMAT);
            convert_layouts.insert(format, layout);
            convert.insert(format, pipeline);
        }
        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewer"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("convert params"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Pipelines { convert_layouts, convert, present_layout, present: HashMap::new(), sampler, params }
    }

    fn present_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        let layout = &self.present_layout;
        self.present.entry(format).or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("present"),
                source: wgpu::ShaderSource::Wgsl(shaders::present().into()),
            });
            render_pipeline(device, layout, &module, format)
        })
    }
}

fn render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Where the last `render` call drew the image, for diagnostics and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderInfo {
    pub image: PhysicalRect,
    pub target_width: u32,
    pub target_height: u32,
}

impl ViewerRenderer {
    pub fn new(ctx: GpuContext) -> Self {
        let pipelines = Pipelines::new(&ctx.device);
        ViewerRenderer {
            ctx,
            pipelines,
            source: None,
            current: None,
            current_transfer: Transfer::Sdr,
            background: wgpu::Color::BLACK,
            stats: UploadStats::default(),
        }
    }

    pub fn context(&self) -> &GpuContext {
        &self.ctx
    }

    /// The frame currently shown (kept for device-loss recovery).
    pub fn current_frame(&self) -> Option<&VideoFrame> {
        self.current.as_ref()
    }

    /// The transfer the current frame was uploaded with.
    pub fn current_transfer(&self) -> Transfer {
        self.current_transfer
    }

    /// Uploads an SDR `frame` (one CPU→GPU copy per plane) and converts it to the intermediate texture.
    pub fn upload(&mut self, frame: &VideoFrame) -> Result<(), GpuError> {
        self.upload_with_transfer(frame, Transfer::Sdr)
    }

    /// Like [`ViewerRenderer::upload`], tone-mapping a PQ / HLG YUV frame to SDR BT.709 in the
    /// conversion pass (the same pipeline as `ove_media::tonemap`). Ignored for RGBA frames.
    pub fn upload_with_transfer(&mut self, frame: &VideoFrame, transfer: Transfer) -> Result<(), GpuError> {
        frame.validate()?;
        if self.ctx.is_lost() {
            return Err(GpuError::DeviceLost(self.ctx.lost_reason()));
        }
        self.ensure_source(frame.format, frame.width, frame.height);
        let Some(source) = self.source.as_ref() else {
            return Err(GpuError::Device("the frame textures were not created".into()));
        };
        let mut bytes = 0u64;
        for (plane, texture) in source.planes.iter().enumerate() {
            let (w, h, _) = frame.format.plane_layout(plane, frame.width, frame.height);
            self.ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.planes[plane],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(frame.strides[plane]),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            bytes += u64::from(frame.strides[plane]) * u64::from(h);
        }

        let (bits, full) = match frame.format {
            PixelFormat::P010 => (10, frame.full_range),
            PixelFormat::Nv12 => (8, frame.full_range),
            PixelFormat::Rgba8 => (8, true),
        };
        let m = yuv_matrix(frame.matrix);
        let r = range_params(bits, full);
        let transfer = if frame.format == PixelFormat::Rgba8 { Transfer::Sdr } else { transfer };
        let (tone, g) = tone_params(transfer);
        let params: [f32; 32] = [
            m[0][0], m[0][1], m[0][2], 0.0, m[1][0], m[1][1], m[1][2], 0.0, m[2][0], m[2][1], m[2][2], 0.0, r[0], r[1],
            r[2], r[3], tone[0], tone[1], tone[2], tone[3], g[0][0], g[0][1], g[0][2], 0.0, g[1][0], g[1][1], g[1][2],
            0.0, g[2][0], g[2][1], g[2][2], 0.0,
        ];
        self.ctx.queue.write_buffer(&self.pipelines.params, 0, bytemuck::cast_slice(&params));

        let view = source.intermediate.create_view(&Default::default());
        let mut encoder = self.ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("convert"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.convert[&frame.format]);
            pass.set_bind_group(0, &source.convert_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.ctx.queue.submit([encoder.finish()]);

        self.stats.frames.fetch_add(1, Ordering::Relaxed);
        self.stats.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.stats.copies_last_frame.store(1, Ordering::Relaxed);
        self.current = Some(frame.clone());
        self.current_transfer = transfer;
        Ok(())
    }

    fn ensure_source(&mut self, format: PixelFormat, width: u32, height: u32) {
        if let Some(s) = &self.source
            && s.format == format
            && s.width == width
            && s.height == height
        {
            return;
        }
        let device = &self.ctx.device;
        let planes: Vec<wgpu::Texture> = (0..format.plane_count())
            .map(|plane| {
                let (w, h, _) = format.plane_layout(plane, width, height);
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("frame plane"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: plane_texture_format(format, plane),
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            })
            .collect();
        let intermediate = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame rgba"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INTERMEDIATE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let plane_views: Vec<wgpu::TextureView> = planes.iter().map(|t| t.create_view(&Default::default())).collect();
        let mut entries =
            vec![wgpu::BindGroupEntry { binding: 0, resource: self.pipelines.params.as_entire_binding() }];
        for (i, view) in plane_views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let convert_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("convert"),
            layout: &self.pipelines.convert_layouts[&format],
            entries: &entries,
        });
        let intermediate_view = intermediate.create_view(&Default::default());
        let present_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.pipelines.present_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&intermediate_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.pipelines.sampler) },
            ],
        });
        self.source = Some(SourceTextures { format, width, height, planes, intermediate, convert_bind, present_bind });
    }

    /// Draws the current frame letterboxed into `target` (whole target = viewer rectangle).
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> RenderInfo {
        let device = self.ctx.device.clone();
        self.pipelines.present_pipeline(&device, format);
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut info = RenderInfo { image: PhysicalRect::default(), target_width: width, target_height: height };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(self.background), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            if let Some(source) = &self.source {
                let fit = fit_rect(source.width, source.height, width, height);
                if fit.width > 0 && fit.height > 0 {
                    pass.set_viewport(fit.x as f32, fit.y as f32, fit.width as f32, fit.height as f32, 0.0, 1.0);
                    pass.set_pipeline(&self.pipelines.present[&format]);
                    pass.set_bind_group(0, &source.present_bind, &[]);
                    pass.draw(0..3, 0..1);
                    info.image = fit;
                }
            }
        }
        self.ctx.queue.submit([encoder.finish()]);
        info
    }

    /// Renders into an offscreen RGBA8 texture and reads it back (tight rows). Used by tests and
    /// by the frame-streaming presenter.
    pub fn render_to_rgba(&mut self, width: u32, height: u32) -> Result<(Vec<u8>, RenderInfo), GpuError> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let texture = self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let info = self.render(&view, format, width, height);

        let row = width * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.ctx.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.ctx.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv().map_err(|e| GpuError::Readback(e.to_string()))?.map_err(|e| GpuError::Readback(e.to_string()))?;
        let mapped = slice.get_mapped_range().map_err(|e| GpuError::Readback(e.to_string()))?;
        let mut out = Vec::with_capacity((row * height) as usize);
        for r in 0..height as usize {
            let start = r * padded as usize;
            out.extend_from_slice(&mapped[start..start + row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Ok((out, info))
    }

    /// If the device was lost, recreates it (on the same instance), rebuilds all GPU resources
    /// and re-uploads the current frame. Returns true when a recovery happened.
    pub fn recover_if_lost(&mut self, surface: Option<&wgpu::Surface<'_>>) -> Result<bool, GpuError> {
        if !self.ctx.is_lost() {
            return Ok(false);
        }
        let reason = self.ctx.lost_reason();
        let ctx = self.ctx.recreate(surface)?;
        tracing::info!(%reason, adapter = %ctx.summary(), generation = ctx.generation, "GPU device recreated");
        self.pipelines = Pipelines::new(&ctx.device);
        self.ctx = ctx;
        self.source = None;
        if let Some(frame) = self.current.take() {
            self.upload_with_transfer(&frame, self.current_transfer)?;
        }
        Ok(true)
    }
}
