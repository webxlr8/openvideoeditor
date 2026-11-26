//! The GPU path for effects: each effect's per-pixel function, ported to WGSL (`wgsl/<id>.wgsl`), runs as a compute
//! shader on the graphics card. The image is uploaded, processed and read back, so the rest of the compositor stays
//! on the CPU.
//!
//! - Used for images of at least [`MIN_PIXELS`] when a hardware adapter opens (its own device, made on first use).
//!   Anything else, or any failure, falls back to the CPU path, which gives the same picture.
//! - `OVE_EFFECTS_GPU=0` turns it off. `OVE_GPU_ALLOW_SOFTWARE=1` lets it use a software adapter (tests on machines
//!   without a GPU).
//! - The WGSL mirrors the Rust: the prelude has the same sampling, hash and noise, and the runner adds one accessor
//!   per parameter (`p_<name>()`), in contract order.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use ove_gpu::wgpu;

use super::spec::{EffectSpec, Group, ParamDefault};
use super::util::Params;
use crate::canvas::Rgba;

/// Smaller images are quicker on the CPU than a round trip to the GPU.
pub const MIN_PIXELS: u32 = 640 * 360;
const MAX_PARAMS: usize = 16;
const PRELUDE: &str = include_str!("wgsl/prelude.wgsl");
const NO_PORT: &str = "no GPU port";

/// The WGSL port of an effect, if it has one (a file still marked `// NOT PORTED` is not).
pub fn wgsl(id: &str) -> Option<&'static str> {
    let body = match id {
        "aurora" => include_str!("wgsl/aurora.wgsl"),
        "bulge" => include_str!("wgsl/bulge.wgsl"),
        "chromatic-aberration" => include_str!("wgsl/chromatic-aberration.wgsl"),
        "conic-gradient" => include_str!("wgsl/conic-gradient.wgsl"),
        "crt-screen" => include_str!("wgsl/crt-screen.wgsl"),
        "dither" => include_str!("wgsl/dither.wgsl"),
        "duotone" => include_str!("wgsl/duotone.wgsl"),
        "engraving" => include_str!("wgsl/engraving.wgsl"),
        "flowing-gradient" => include_str!("wgsl/flowing-gradient.wgsl"),
        "fluted-glass" => include_str!("wgsl/fluted-glass.wgsl"),
        "glass-tiles" => include_str!("wgsl/glass-tiles.wgsl"),
        "glitch" => include_str!("wgsl/glitch.wgsl"),
        "godrays" => include_str!("wgsl/godrays.wgsl"),
        "gradient-map" => include_str!("wgsl/gradient-map.wgsl"),
        "halftone" => include_str!("wgsl/halftone.wgsl"),
        "kaleidoscope" => include_str!("wgsl/kaleidoscope.wgsl"),
        "lens-distortion" => include_str!("wgsl/lens-distortion.wgsl"),
        "lens-flare" => include_str!("wgsl/lens-flare.wgsl"),
        "light-leak" => include_str!("wgsl/light-leak.wgsl"),
        "marble" => include_str!("wgsl/marble.wgsl"),
        "mesh-gradient" => include_str!("wgsl/mesh-gradient.wgsl"),
        "mirror" => include_str!("wgsl/mirror.wgsl"),
        "pixelate" => include_str!("wgsl/pixelate.wgsl"),
        "plasma" => include_str!("wgsl/plasma.wgsl"),
        "polar-coordinates" => include_str!("wgsl/polar-coordinates.wgsl"),
        "posterize" => include_str!("wgsl/posterize.wgsl"),
        "solarize" => include_str!("wgsl/solarize.wgsl"),
        "spherize" => include_str!("wgsl/spherize.wgsl"),
        "spiral" => include_str!("wgsl/spiral.wgsl"),
        "stripes" => include_str!("wgsl/stripes.wgsl"),
        "studio-background" => include_str!("wgsl/studio-background.wgsl"),
        "sunburst" => include_str!("wgsl/sunburst.wgsl"),
        "swirl" => include_str!("wgsl/swirl.wgsl"),
        "tilt-shift" => include_str!("wgsl/tilt-shift.wgsl"),
        "tritone" => include_str!("wgsl/tritone.wgsl"),
        "twirl" => include_str!("wgsl/twirl.wgsl"),
        "vhs" => include_str!("wgsl/vhs.wgsl"),
        "voronoi" => include_str!("wgsl/voronoi.wgsl"),
        "watercolor" => include_str!("wgsl/watercolor.wgsl"),
        "wave-distortion" => include_str!("wgsl/wave-distortion.wgsl"),
        "zoom-blur" => include_str!("wgsl/zoom-blur.wgsl"),
        _ => return None,
    };
    (!body.starts_with("// NOT PORTED")).then_some(body)
}

/// The full shader for an effect: prelude, parameter accessors, the port, and the entry point.
pub fn shader_source(spec: &EffectSpec) -> Option<String> {
    let body = wgsl(spec.id)?;
    let mut s = String::from(PRELUDE);
    s.push('\n');
    for (i, p) in spec.params.iter().enumerate() {
        let name: String = p.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let (ty, get) = match p.default {
            ParamDefault::Num(_) => ("f32", "x"),
            ParamDefault::Vec2(_) => ("vec2<f32>", "xy"),
            ParamDefault::Vec3(_) => ("vec3<f32>", "xyz"),
        };
        s.push_str(&format!("fn p_{name}() -> {ty} {{ return params[{i}].{get}; }}\n"));
    }
    s.push_str(body);
    s.push_str(MAIN);
    Some(s)
}

/// Mixes the effect into the source by `amount` exactly as `effects::apply` does on the CPU.
const MAIN: &str = r#"
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u32(info.w);
    let h = u32(info.h);
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let i = gid.y * w + gid.x;
    let o = effect((f32(gid.x) + 0.5) / info.w, (f32(gid.y) + 0.5) / info.h);
    let p = src[i];
    var out = 0u;
    for (var k = 0u; k < 3u; k = k + 1u) {
        let s = f32((p >> (8u * k)) & 0xffu) / 255.0;
        var val = o[k];
        if (!finite(val)) {
            val = s;
        }
        let b = u32(clamp(s + (val - s) * info.amount, 0.0, 1.0) * 255.0 + 0.5);
        out = out | (min(b, 255u) << (8u * k));
    }
    var a = (p >> 24u) & 0xffu;
    if (info.generator > 0.5) {
        a = u32(f32(a) + (255.0 - f32(a)) * info.amount + 0.5);
    }
    dst[i] = out | (min(a, 255u) << 24u);
}
"#;

struct Buffers {
    bytes: u64,
    src: wgpu::Buffer,
    dst: wgpu::Buffer,
    read: wgpu::Buffer,
}

struct Engine {
    ctx: ove_gpu::GpuContext,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<&'static str, Result<wgpu::ComputePipeline, String>>,
    info: wgpu::Buffer,
    params: wgpu::Buffer,
    buffers: Option<Buffers>,
}

static ENGINE: OnceLock<Option<Mutex<Engine>>> = OnceLock::new();
/// Set after a failure: the GPU path stays off for the rest of the run.
static BROKEN: AtomicBool = AtomicBool::new(false);

fn enabled() -> bool {
    std::env::var("OVE_EFFECTS_GPU").map_or(true, |v| v != "0") && !BROKEN.load(Ordering::Relaxed)
}

fn engine() -> Option<&'static Mutex<Engine>> {
    ENGINE
        .get_or_init(|| {
            let allow_software = std::env::var("OVE_GPU_ALLOW_SOFTWARE").is_ok_and(|v| v == "1");
            match ove_gpu::GpuContext::new(ove_gpu::GpuOptions { allow_software }) {
                Ok(ctx) => {
                    tracing::info!(adapter = %ctx.summary().name, "effects run on the GPU");
                    Some(Mutex::new(Engine::new(ctx)))
                }
                Err(e) => {
                    tracing::info!("effects run on the CPU: {e}");
                    None
                }
            }
        })
        .as_ref()
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Engine {
    fn new(ctx: ove_gpu::GpuContext) -> Engine {
        let device = &ctx.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effects"),
            entries: &[storage(0, true), storage(1, false), uniform(2), uniform(3)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effects"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let uniform_buffer = |size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("effects uniforms"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let info = uniform_buffer(32);
        let params = uniform_buffer((MAX_PARAMS * 16) as u64);
        Engine { ctx, layout, pipeline_layout, pipelines: HashMap::new(), info, params, buffers: None }
    }

    /// The compiled pipeline of an effect, or why there is none (no port, or its shader does not compile).
    fn pipeline(&mut self, spec: &'static EffectSpec) -> Result<&wgpu::ComputePipeline, String> {
        if !self.pipelines.contains_key(spec.id) {
            let built = shader_source(spec).ok_or_else(|| NO_PORT.to_string()).and_then(|src| {
                let device = &self.ctx.device;
                let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(spec.id),
                    source: wgpu::ShaderSource::Wgsl(src.into()),
                });
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(spec.id),
                    layout: Some(&self.pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                match pollster::block_on(scope.pop()) {
                    None => Ok(pipeline),
                    Some(e) => {
                        tracing::warn!(effect = spec.id, "the GPU port does not compile, using the CPU: {e}");
                        Err(format!("{NO_PORT}: the shader does not compile: {e}"))
                    }
                }
            });
            self.pipelines.insert(spec.id, built);
        }
        match self.pipelines.get(spec.id) {
            Some(Ok(p)) => Ok(p),
            Some(Err(e)) => Err(e.clone()),
            None => Err(NO_PORT.to_string()),
        }
    }

    /// Buffers for images of `bytes`, kept while the size stays the same.
    fn buffers<'a>(slot: &'a mut Option<Buffers>, device: &wgpu::Device, bytes: u64) -> &'a Buffers {
        if slot.as_ref().is_some_and(|b| b.bytes != bytes) {
            *slot = None;
        }
        slot.get_or_insert_with(|| {
            let make = |label, usage| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage,
                    mapped_at_creation: false,
                })
            };
            Buffers {
                bytes,
                src: make("effects src", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
                dst: make("effects dst", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
                read: make("effects read", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            }
        })
    }

    fn run(
        &mut self,
        img: &mut Rgba,
        spec: &'static EffectSpec,
        params: &Params,
        amount: f32,
        t: f32,
    ) -> Result<(), String> {
        if spec.params.len() > MAX_PARAMS {
            return Err(format!("{} has more than {MAX_PARAMS} parameters", spec.id));
        }
        let pipeline = self.pipeline(spec)?.clone();
        let bytes = img.data.len() as u64;
        let max = self.ctx.device.limits().max_storage_buffer_binding_size;
        if bytes == 0 || bytes > max || !bytes.is_multiple_of(4) {
            return Err(format!("{bytes} bytes do not fit a storage buffer"));
        }
        let info = [
            img.w as f32,
            img.h as f32,
            img.w as f32 / img.h as f32,
            if t.is_finite() { t } else { 0.0 },
            amount,
            if spec.group == Group::Background { 1.0 } else { 0.0 },
            0.0,
            0.0,
        ];
        let mut packed = params.packed(spec);
        packed.resize(MAX_PARAMS, [0.0; 4]);
        let queue = self.ctx.queue.clone();
        queue.write_buffer(&self.info, 0, bytemuck::cast_slice(&info));
        queue.write_buffer(&self.params, 0, bytemuck::cast_slice(packed.as_flattened()));
        let device = self.ctx.device.clone();
        let b = Self::buffers(&mut self.buffers, &device, bytes);
        queue.write_buffer(&b.src, 0, &img.data);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effects"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: b.src.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b.dst.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.info.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.params.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(spec.id), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(img.w.div_ceil(8), img.h.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&b.dst, 0, &b.read, 0, bytes);
        queue.submit([encoder.finish()]);
        let slice = b.read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        if self.ctx.is_lost() {
            return Err("the GPU device was lost".into());
        }
        // Only now is the image changed: every failure above leaves it as it was, for the CPU path.
        {
            let mapped = slice.get_mapped_range().map_err(|e| e.to_string())?;
            img.data.copy_from_slice(&mapped);
        }
        b.read.unmap();
        Ok(())
    }
}

/// Runs the effect on the GPU when that is on and worth it. False leaves `img` untouched for the CPU path.
pub(super) fn try_apply(img: &mut Rgba, spec: &'static EffectSpec, params: &Params, amount: f32, t: f32) -> bool {
    if img.w.saturating_mul(img.h) < MIN_PIXELS || wgsl(spec.id).is_none() || !enabled() {
        return false;
    }
    run(img, spec, params, amount, t).is_ok()
}

/// Runs the effect on the GPU whatever the image size (tests compare it with the CPU).
pub fn run(img: &mut Rgba, spec: &'static EffectSpec, params: &Params, amount: f32, t: f32) -> Result<(), String> {
    let engine = engine().ok_or_else(|| "no GPU".to_string())?;
    let mut e = engine.lock().unwrap_or_else(PoisonError::into_inner);
    let r = e.run(img, spec, params, amount, t);
    if let Err(err) = &r
        && !err.starts_with(NO_PORT)
    {
        tracing::warn!(effect = spec.id, "GPU effects turned off: {err}");
        BROKEN.store(true, Ordering::Relaxed);
    }
    r
}

/// Whether a GPU opened for effects (opens it on first call).
pub fn available() -> bool {
    enabled() && engine().is_some()
}
