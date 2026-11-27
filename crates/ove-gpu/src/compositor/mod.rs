//! The GPU compositing backend (design D1): `ove-render`'s [`Backend`] on wgpu compute shaders.
//!
//! Images are `rgba8unorm` storage textures; every operation reads its inputs and writes a fresh output texture
//! (ping-pong), rounding to 8 bits exactly where the CPU reference does, so the CPU backend stays the oracle and the
//! two agree within the parity tolerance. Layer drawing with all 16 blend modes, the basic grade, LUTs and baked
//! grade lattices, partial-strength mixing and the final alpha pass run natively. Every other operation (effects,
//! transition masks, captions) falls back per layer: the image is read back, the CPU code runs, and the result is
//! uploaded (design D2).

mod effects;
mod select;
mod shaders;

pub use effects::PORTED as NATIVE_EFFECTS;

pub use select::{Choice, SelectOptions, Selection, select, select_with, self_test};

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use ove_render::backend::{Backend, Slab, Tex};
use ove_render::canvas::{DrawParams, Place, Rgba};
use ove_render::color::Lut;
use ove_render::eval::{Blend, Grade};
use wgpu::util::DeviceExt;

use crate::renderer::{AdapterSummary, GpuContext, GpuError, GpuOptions};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Largest LUT (per side) uploaded to the GPU; larger ones run on the CPU.
const MAX_LUT_SIDE: usize = 65;

struct Image {
    tex: wgpu::Texture,
    w: u32,
    h: u32,
}

struct Pipes {
    draw: wgpu::ComputePipeline,
    grade: wgpu::ComputePipeline,
    lut: wgpu::ComputePipeline,
    mix: wgpu::ComputePipeline,
    opaque: wgpu::ComputePipeline,
}

fn pipeline(device: &wgpu::Device, label: &str, source: String) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// A pipeline whose shader is checked: `None` (logged) when the driver rejects it, so the caller can fall back to
/// the CPU instead of producing empty frames.
fn checked_pipeline(device: &wgpu::Device, label: &str, source: String) -> Option<wgpu::ComputePipeline> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let p = pipeline(device, label, source);
    match pollster::block_on(scope.pop()) {
        None => Some(p),
        Some(e) => {
            tracing::warn!(shader = label, "GPU shader rejected; using the CPU for it: {e}");
            None
        }
    }
}

impl Pipes {
    fn new(device: &wgpu::Device) -> Pipes {
        Pipes {
            draw: pipeline(device, "compositor draw", shaders::draw()),
            grade: pipeline(device, "compositor grade", shaders::grade()),
            lut: pipeline(device, "compositor lut", shaders::lut()),
            mix: pipeline(device, "compositor mix", shaders::mix()),
            opaque: pipeline(device, "compositor opaque", shaders::opaque()),
        }
    }
}

/// Counters for the diagnostics overlay and the readback-cost risk (design: Risks).
#[derive(Debug, Default)]
pub struct Counters {
    /// Compute passes run.
    pub passes: AtomicU64,
    /// Operations that fell back to the CPU (readback, CPU code, upload).
    pub cpu_fallbacks: AtomicU64,
    /// Bytes read back from the GPU.
    pub readback_bytes: AtomicU64,
    /// Bytes uploaded to the GPU.
    pub upload_bytes: AtomicU64,
}

/// One binding of a compute pass.
enum Bind<'a> {
    Buffer(&'a wgpu::Buffer),
    Texture(&'a wgpu::TextureView),
}

/// `blendMode` as the shader's mode number.
fn mode(b: Blend) -> u32 {
    match b {
        Blend::Normal => 0,
        Blend::Multiply => 1,
        Blend::Screen => 2,
        Blend::Overlay => 3,
        Blend::Darken => 4,
        Blend::Lighten => 5,
        Blend::ColorDodge => 6,
        Blend::ColorBurn => 7,
        Blend::HardLight => 8,
        Blend::SoftLight => 9,
        Blend::Difference => 10,
        Blend::Exclusion => 11,
        Blend::Hue => 12,
        Blend::Saturation => 13,
        Blend::Color => 14,
        Blend::Luminosity => 15,
        Blend::Add => 16,
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The GPU compositing backend.
pub struct GpuBackend {
    ctx: GpuContext,
    pipes: Pipes,
    images: Slab<Image>,
    /// Spare textures by size, reused as ping-pong outputs.
    pool: HashMap<(u32, u32), Vec<wgpu::Texture>>,
    /// Uploaded lattices by the address of their `Arc<Lut>`.
    luts: HashMap<usize, (Weak<Lut>, wgpu::Buffer)>,
    /// Ported effects' pipelines, built on first use (`None`: the driver rejected the shader; CPU fallback).
    fx: HashMap<&'static str, Option<wgpu::ComputePipeline>>,
    /// The transition mask pipeline, built on first use (inner `None`: rejected by the driver; CPU fallback).
    mask: Option<Option<wgpu::ComputePipeline>>,
    /// The scope binning pipeline, built on first use.
    scope_pipe: Option<Option<wgpu::ComputePipeline>>,
    pub counters: Arc<Counters>,
}

impl GpuBackend {
    /// A backend on an opened GPU.
    pub fn new(ctx: GpuContext) -> GpuBackend {
        let pipes = Pipes::new(&ctx.device);
        GpuBackend {
            ctx,
            pipes,
            images: Slab::default(),
            pool: HashMap::new(),
            luts: HashMap::new(),
            fx: HashMap::new(),
            mask: None,
            scope_pipe: None,
            counters: Arc::new(Counters::default()),
        }
    }

    /// Opens the GPU and builds the backend.
    pub fn open(options: GpuOptions) -> Result<GpuBackend, GpuError> {
        GpuContext::new(options).map(GpuBackend::new)
    }

    pub fn adapter(&self) -> AdapterSummary {
        self.ctx.summary()
    }

    pub fn context(&self) -> &GpuContext {
        &self.ctx
    }

    /// A handle that simulates a device loss after the backend has moved into a renderer (tests, diagnostics).
    pub fn loss_handle(&self) -> crate::renderer::LossHandle {
        self.ctx.loss_handle()
    }

    /// The device was lost (driver reset, removal); the backend cannot be used any more.
    pub fn is_lost(&self) -> bool {
        self.ctx.is_lost()
    }

    /// Waits until the GPU has finished all submitted work (timings).
    pub fn finish(&self) {
        let _ = self.ctx.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// The counts of `scope` for image `t` (`ove_render::scopes::SIZE`² bins), from a compute pass with atomic
    /// counters; the same integer binning as `ove_render::scopes::bin`. `None` when the GPU cannot run it.
    pub fn scope(&mut self, t: Tex, scope: ove_render::scopes::Scope) -> Option<Vec<u32>> {
        let n = ove_render::scopes::SIZE * ove_render::scopes::SIZE;
        let img = self.images.get(t)?;
        let (w, h) = (img.w, img.h);
        let view = img.tex.create_view(&Default::default());
        let pipeline = match &self.scope_pipe {
            Some(p) => p.clone()?,
            None => {
                let p = checked_pipeline(&self.ctx.device, "scope", effects::SCOPE.to_string());
                self.scope_pipe = Some(p.clone());
                p?
            }
        };
        let kind = match scope {
            ove_render::scopes::Scope::Waveform => 0u32,
            ove_render::scopes::Scope::Parade => 1,
            ove_render::scopes::Scope::Vectorscope => 2,
        };
        let head = self.uniform(&[kind, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        let bytes = (n * 4) as u64;
        let counts = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope counts"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let group = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: head.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: counts.as_entire_binding() },
            ],
        });
        let mut encoder = self.ctx.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&counts, 0, None);
        {
            let mut pass = encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("scope"), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&counts, 0, &readback, 0, bytes);
        self.ctx.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.ctx.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let mapped = slice.get_mapped_range().ok()?;
        let out = mapped.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        drop(mapped);
        readback.unmap();
        Some(out)
    }

    /// Images currently held.
    pub fn held(&self) -> usize {
        self.images.len()
    }

    /// Rebuilds the layer-drawing shader from edited source. For tests that check a wrong shader is caught by the
    /// parity harness; never used by the app.
    #[doc(hidden)]
    pub fn rebuild_draw_shader(&mut self, edit: impl Fn(String) -> String) {
        self.pipes.draw = pipeline(&self.ctx.device, "compositor draw (edited)", edit(shaders::draw()));
    }

    fn texture(&mut self, w: u32, h: u32) -> wgpu::Texture {
        let (w, h) = (w.max(1), h.max(1));
        if let Some(t) = self.pool.get_mut(&(w, h)).and_then(Vec::pop) {
            return t;
        }
        self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("compositor image"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn recycle(&mut self, tex: wgpu::Texture) {
        let key = (tex.width(), tex.height());
        let spare = self.pool.entry(key).or_default();
        if spare.len() < 4 {
            spare.push(tex);
        }
    }

    fn write(&self, tex: &wgpu::Texture, img: &Rgba) {
        if img.w == 0 || img.h == 0 {
            return;
        }
        self.ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &img.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.w * 4), rows_per_image: Some(img.h) },
            wgpu::Extent3d { width: img.w, height: img.h, depth_or_array_layers: 1 },
        );
        self.counters.upload_bytes.fetch_add(img.data.len() as u64, Ordering::Relaxed);
    }

    fn insert(&mut self, img: &Rgba) -> Tex {
        let tex = self.texture(img.w, img.h);
        self.write(&tex, img);
        self.images.insert(Image { tex, w: img.w, h: img.h })
    }

    fn read_texture(&self, tex: &wgpu::Texture, w: u32, h: u32) -> Result<Rgba, GpuError> {
        if w == 0 || h == 0 {
            return Ok(Rgba::new(w, h));
        }
        let row = w * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compositor readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.ctx.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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
        let mut data = Vec::with_capacity((row * h) as usize);
        for r in 0..h as usize {
            let start = r * padded as usize;
            data.extend_from_slice(&mapped[start..start + row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        self.counters.readback_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
        Ok(Rgba { w, h, data })
    }

    /// Runs `pipe` over a `w`×`h` output with the given bindings (the output storage texture is added as the last
    /// binding), and returns the output texture.
    fn pass(&mut self, pipe: Pipe, binds: &[(u32, Bind<'_>)], out_binding: u32, w: u32, h: u32) -> wgpu::Texture {
        let pipeline = match pipe {
            Pipe::Draw => self.pipes.draw.clone(),
            Pipe::Grade => self.pipes.grade.clone(),
            Pipe::Lut => self.pipes.lut.clone(),
            Pipe::Mix => self.pipes.mix.clone(),
            Pipe::Opaque => self.pipes.opaque.clone(),
            Pipe::Other(p) => p,
        };
        let pipeline = &pipeline;
        let out = self.texture(w, h);
        let out_view = out.create_view(&Default::default());
        let mut entries: Vec<wgpu::BindGroupEntry<'_>> = binds
            .iter()
            .map(|(b, r)| wgpu::BindGroupEntry {
                binding: *b,
                resource: match r {
                    Bind::Buffer(buf) => buf.as_entire_binding(),
                    Bind::Texture(v) => wgpu::BindingResource::TextureView(v),
                },
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: out_binding,
            resource: wgpu::BindingResource::TextureView(&out_view),
        });
        let group = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor pass"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut encoder = self.ctx.device.create_command_encoder(&Default::default());
        {
            let mut cpass = encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("compositor"), timestamp_writes: None });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &group, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        self.ctx.queue.submit([encoder.finish()]);
        self.counters.passes.fetch_add(1, Ordering::Relaxed);
        out
    }

    fn uniform(&self, bytes: &[u8]) -> wgpu::Buffer {
        self.ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("compositor params"),
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    fn storage(&self, bytes: &[u8]) -> wgpu::Buffer {
        self.ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("compositor data"),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        })
    }

    /// Replaces the texture of `t` with `tex` (the old one goes back to the pool).
    fn replace(&mut self, t: Tex, tex: wgpu::Texture) {
        let old = match self.images.get_mut(t) {
            Some(img) => std::mem::replace(&mut img.tex, tex),
            None => tex,
        };
        self.recycle(old);
    }

    /// Runs a one-input pass (`src` at binding 1, output at 2, parameters at 0, extra data at 3) on `t` in place.
    fn unary(&mut self, t: Tex, pipe: Pipe, params: Option<&wgpu::Buffer>, data: Option<&wgpu::Buffer>) {
        let Some(img) = self.images.get(t) else { return };
        let (w, h) = (img.w, img.h);
        if w == 0 || h == 0 {
            return;
        }
        let view = img.tex.create_view(&Default::default());
        let mut binds = vec![(1, Bind::Texture(&view))];
        if let Some(p) = params {
            binds.push((0, Bind::Buffer(p)));
        }
        if let Some(d) = data {
            binds.push((3, Bind::Buffer(d)));
        }
        let out = self.pass(pipe, &binds, 2, w, h);
        self.replace(t, out);
    }

    /// The lattice of `lut` on the GPU (cached while the `Arc` lives).
    fn lattice(&mut self, lut: &Arc<Lut>) -> wgpu::Buffer {
        self.luts.retain(|_, (w, _)| w.strong_count() > 0);
        let key = Arc::as_ptr(lut) as usize;
        if let Some((w, b)) = self.luts.get(&key)
            && w.upgrade().is_some_and(|l| Arc::ptr_eq(&l, lut))
        {
            return b.clone();
        }
        let flat: Vec<f32> = lut.data.iter().flat_map(|c| [c[0], c[1], c[2], 0.0]).collect();
        let buf = self.storage(&f32s(&flat));
        self.luts.insert(key, (Arc::downgrade(lut), buf.clone()));
        buf
    }
}

#[derive(Clone)]
enum Pipe {
    Draw,
    Grade,
    Lut,
    Mix,
    Opaque,
    /// A ported effect or the transition mask.
    Other(wgpu::ComputePipeline),
}

/// The destination pixels a draw touches (as `canvas::draw` computes them), or `None` when it draws nothing.
fn draw_box(dw: u32, dh: u32, sw: f64, sh: f64, place: &Place, crop: [f64; 4]) -> Option<[u32; 4]> {
    let [crop_l, crop_r, crop_t, crop_b] = crop;
    let th = place.rotation.to_radians();
    let (sin, cos) = th.sin_cos();
    let corners = [(crop_l, crop_t), (crop_r, crop_t), (crop_l, crop_b), (crop_r, crop_b)];
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (u, v) in corners {
        let (lx, ly) = ((u - sw / 2.0) * place.sx, (v - sh / 2.0) * place.sy);
        let (x, y) = (lx * cos - ly * sin + place.cx, lx * sin + ly * cos + place.cy);
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let x0 = minx.floor().max(0.0) as u32;
    let y0 = miny.floor().max(0.0) as u32;
    let x1 = (maxx.ceil().max(0.0) as u32).min(dw);
    let y1 = (maxy.ceil().max(0.0) as u32).min(dh);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// For each row of `bbox`, the destination pixels `[x0, x1)` whose centre maps inside the cropped source, decided
/// with the reference's own f64 arithmetic (`canvas::draw`). The mapped area is convex, so each row is one span.
fn draw_spans(bbox: [u32; 4], sw: f64, sh: f64, place: &Place, crop: [f64; 4]) -> Vec<[u32; 2]> {
    let [crop_l, crop_r, crop_t, crop_b] = crop;
    let (sin, cos) = place.rotation.to_radians().sin_cos();
    let uv = |x: u32, dy: f64| {
        let dx = x as f64 + 0.5 - place.cx;
        let rx = dx * cos + dy * sin;
        let ry = -dx * sin + dy * cos;
        (rx / place.sx + sw / 2.0, ry / place.sy + sh / 2.0)
    };
    let inside = |x: u32, dy: f64| {
        let (u, v) = uv(x, dy);
        !(u < crop_l || u >= crop_r || v < crop_t || v >= crop_b)
    };
    let [bx0, by0, bx1, by1] = bbox;
    let mut out = Vec::with_capacity((by1 - by0) as usize);
    for y in by0..by1 {
        let dy = y as f64 + 0.5 - place.cy;
        // u and v are linear in x: x-ranges from the four edges, then exact checks around the ends.
        let (au, bu) = (cos / place.sx, ((0.5 - place.cx) * cos + dy * sin) / place.sx + sw / 2.0);
        let (av, bv) = (-sin / place.sy, (-(0.5 - place.cx) * sin + dy * cos) / place.sy + sh / 2.0);
        let (mut lo, mut hi) = (bx0 as f64, bx1 as f64);
        for (a, b, min, max) in [(au, bu, crop_l, crop_r), (av, bv, crop_t, crop_b)] {
            if a.abs() < 1e-12 {
                if b < min || b >= max {
                    hi = lo;
                }
            } else {
                let (e0, e1) = ((min - b) / a, (max - b) / a);
                lo = lo.max(e0.min(e1));
                hi = hi.min(e0.max(e1));
            }
        }
        let clamp = |v: f64| v.clamp(bx0 as f64, bx1 as f64) as u32;
        let (mut x0, mut x1) = (clamp(lo.floor() - 1.0), clamp(hi.ceil() + 1.0));
        while x0 < x1 && !inside(x0, dy) {
            x0 += 1;
        }
        while x1 > x0 && !inside(x1 - 1, dy) {
            x1 -= 1;
        }
        out.push([x0, x1]);
    }
    out
}

impl Backend for GpuBackend {
    fn name(&self) -> &'static str {
        "gpu"
    }

    fn create(&mut self, w: u32, h: u32, fill: [u8; 4]) -> Tex {
        let img = if fill == [0; 4] { Rgba::new(w, h) } else { Rgba::filled(w, h, fill) };
        self.insert(&img)
    }

    fn upload(&mut self, img: &Arc<Rgba>) -> Tex {
        self.insert(img)
    }

    fn read(&mut self, t: Tex) -> Rgba {
        let Some(img) = self.images.get(t) else { return Rgba::new(0, 0) };
        match self.read_texture(&img.tex, img.w, img.h) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("GPU readback failed: {e}");
                Rgba::new(img.w, img.h)
            }
        }
    }

    fn release(&mut self, t: Tex) {
        if let Some(img) = self.images.remove(t) {
            self.recycle(img.tex);
        }
    }

    fn size(&self, t: Tex) -> (u32, u32) {
        self.images.get(t).map(|i| (i.w, i.h)).unwrap_or((0, 0))
    }

    fn healthy(&self) -> bool {
        !self.ctx.is_lost()
    }

    fn cpu(&mut self, t: Tex, f: &mut dyn FnMut(&mut Rgba)) {
        let mut img = self.read(t);
        f(&mut img);
        self.counters.cpu_fallbacks.fetch_add(1, Ordering::Relaxed);
        match self.images.get(t).map(|i| (i.w, i.h)) {
            Some((w, h)) if (w, h) == (img.w, img.h) => {
                let tex = self.texture(w, h);
                self.write(&tex, &img);
                self.replace(t, tex);
            }
            Some(_) => {
                let tex = self.texture(img.w, img.h);
                self.write(&tex, &img);
                if let Some(slot) = self.images.get_mut(t) {
                    let old = std::mem::replace(slot, Image { tex, w: img.w, h: img.h });
                    self.recycle(old.tex);
                }
            }
            None => {}
        }
    }

    fn copy(&mut self, t: Tex) -> Tex {
        let Some((w, h)) = self.images.get(t).map(|i| (i.w, i.h)) else { return self.create(0, 0, [0; 4]) };
        let out = self.texture(w, h);
        if let Some(img) = self.images.get(t) {
            let mut encoder = self.ctx.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_texture(
                img.tex.as_image_copy(),
                out.as_image_copy(),
                wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            );
            self.ctx.queue.submit([encoder.finish()]);
        }
        self.images.insert(Image { tex: out, w, h })
    }

    fn draw(&mut self, dst: Tex, src: Tex, place: Place, p: &DrawParams) {
        if dst == src {
            let copy = self.copy(src);
            self.draw(dst, copy, place, p);
            self.release(copy);
            return;
        }
        let (Some(d), Some(s)) = (self.images.get(dst), self.images.get(src)) else { return };
        let (dw, dh, sw, sh) = (d.w, d.h, s.w as f64, s.h as f64);
        if s.w == 0 || s.h == 0 || dw == 0 || dh == 0 || p.opacity <= 0.0 || place.sx == 0.0 || place.sy == 0.0 {
            return;
        }
        let crop = [
            sw * p.crop[3] as f64 / 100.0,
            sw - sw * p.crop[1] as f64 / 100.0,
            sh * p.crop[0] as f64 / 100.0,
            sh - sh * p.crop[2] as f64 / 100.0,
        ];
        if crop[1] <= crop[0] || crop[3] <= crop[2] {
            return;
        }
        let Some(bbox) = draw_box(dw, dh, sw, sh, &place, crop) else { return };
        let (sin, cos) = place.rotation.to_radians().sin_cos();
        let mut bytes = f32s(&[
            place.cx as f32,
            place.cy as f32,
            place.sx as f32,
            place.sy as f32,
            cos as f32,
            sin as f32,
            sw as f32,
            sh as f32,
            crop[0] as f32,
            crop[1] as f32,
            crop[2] as f32,
            crop[3] as f32,
            p.radius.max(0.0),
            p.opacity,
            mode(p.blend) as f32,
            0.0,
        ]);
        bytes.extend(bbox.iter().flat_map(|v| v.to_le_bytes()));
        let params = self.uniform(&bytes);
        let spans: Vec<u8> =
            draw_spans(bbox, sw, sh, &place, crop).iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let spans = self.storage(&spans);
        let dv = d.tex.create_view(&Default::default());
        let sv = s.tex.create_view(&Default::default());
        let out = self.pass(
            Pipe::Draw,
            &[(0, Bind::Buffer(&params)), (1, Bind::Texture(&dv)), (2, Bind::Texture(&sv)), (4, Bind::Buffer(&spans))],
            3,
            dw,
            dh,
        );
        self.replace(dst, out);
    }

    fn grade(&mut self, t: Tex, g: &Grade) {
        let tables = ove_render::canvas::grade_tables(g);
        let flat: Vec<u8> = tables.iter().flatten().flat_map(|&v| u32::from(v).to_le_bytes()).collect();
        let data = self.storage(&flat);
        let params = self.uniform(&f32s(&[1.0 + g.saturation, g.vibrance, 0.0, 0.0]));
        self.unary(t, Pipe::Grade, Some(&params), Some(&data));
    }

    fn lut(&mut self, t: Tex, lut: &Arc<Lut>, intensity: f32) {
        let k = if intensity.is_finite() { intensity.clamp(0.0, 1.0) } else { 0.0 };
        if k <= 0.0 {
            return;
        }
        if lut.three_d && lut.size > MAX_LUT_SIDE {
            self.cpu(t, &mut |img| ove_render::color::apply_lut(img, lut, k));
            return;
        }
        let data = self.lattice(lut);
        let (a, b) = (lut.domain_min, lut.domain_max);
        let params = self.uniform(&f32s(&[
            a[0],
            a[1],
            a[2],
            0.0,
            b[0],
            b[1],
            b[2],
            1.0,
            lut.size as f32,
            if lut.three_d { 1.0 } else { 0.0 },
            k,
            0.0,
        ]));
        self.unary(t, Pipe::Lut, Some(&params), Some(&data));
    }

    fn mix(&mut self, dst: Tex, before: Tex, k: f32) {
        let (Some(d), Some(b)) = (self.images.get(dst), self.images.get(before)) else { return };
        let (w, h) = (d.w, d.h);
        if (b.w, b.h) != (w, h) || w == 0 || h == 0 {
            return;
        }
        let params = self.uniform(&f32s(&[k, 0.0, 0.0, 0.0]));
        let dv = d.tex.create_view(&Default::default());
        let bv = b.tex.create_view(&Default::default());
        let out = self.pass(
            Pipe::Mix,
            &[(0, Bind::Buffer(&params)), (1, Bind::Texture(&dv)), (2, Bind::Texture(&bv))],
            3,
            w,
            h,
        );
        self.replace(dst, out);
    }

    fn opaque(&mut self, t: Tex) {
        self.unary(t, Pipe::Opaque, None, None);
    }

    fn effect(
        &mut self,
        t: Tex,
        id: &str,
        values: &serde_json::Map<String, serde_json::Value>,
        amount: f32,
        time: f32,
    ) -> bool {
        let Some(spec) = ove_render::effects::find(id) else { return false };
        let native = effects::PORTED.iter().find(|p| **p == id).copied();
        let pipeline = native.and_then(|id| match self.fx.get(id) {
            Some(p) => p.clone(),
            None => {
                let p = checked_pipeline(&self.ctx.device, id, effects::source(id).unwrap_or_default());
                self.fx.insert(id, p.clone());
                p
            }
        });
        let Some(pipeline) = pipeline else {
            // Not ported (or rejected by the driver): the CPU code on this layer (readback, effect, upload).
            let mut known = false;
            self.cpu(t, &mut |img| known = ove_render::effects::apply(img, id, values, amount, time));
            return known;
        };
        let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
        let (w, h) = self.size(t);
        if amount <= 0.0 || w == 0 || h == 0 {
            return true;
        }
        let mut packed = ove_render::effects::Params::resolve(spec, values).packed(spec);
        packed.push(0.0); // a storage buffer is never empty
        let time = if time.is_finite() { time } else { 0.0 };
        let generator = spec.group == ove_render::effects::Group::Background;
        let head = self.uniform(&f32s(&[
            w as f32,
            h as f32,
            w as f32 / h as f32,
            time,
            amount,
            if generator { 1.0 } else { 0.0 },
            0.0,
            0.0,
        ]));
        let data = self.storage(&f32s(&packed));
        self.unary(t, Pipe::Other(pipeline), Some(&head), Some(&data));
        true
    }

    fn transition_mask(&mut self, t: Tex, st: &ove_render::transitions::TransState) {
        if !st.has_mask() {
            return;
        }
        let (base, dir) = ove_render::transitions::split(&st.kind, None);
        let kind = match base {
            "wipe" => 0.0,
            "radial-wipe" => 1.0,
            "ink-bleed" => 2.0,
            _ => 3.0,
        };
        let dir = match dir {
            ove_render::transitions::Dir::Left => 0.0,
            ove_render::transitions::Dir::Right => 1.0,
            ove_render::transitions::Dir::Up => 2.0,
            ove_render::transitions::Dir::Down => 3.0,
        };
        let pipeline = match &self.mask {
            Some(p) => p.clone(),
            None => {
                let p = checked_pipeline(&self.ctx.device, "transition mask", effects::MASK.to_string());
                self.mask = Some(p.clone());
                p
            }
        };
        let Some(pipeline) = pipeline else {
            self.cpu(t, &mut |img| st.apply_mask(img));
            return;
        };
        let head = self.uniform(&f32s(&[kind, dir, st.q, 0.0]));
        self.unary(t, Pipe::Other(pipeline), Some(&head), None);
    }
}
