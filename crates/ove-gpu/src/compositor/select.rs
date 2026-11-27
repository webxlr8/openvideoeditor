//! Backend selection (design D9). The GPU backend is used when a hardware adapter is found and a startup self-test
//! frame matches the CPU reference within the parity tolerance; otherwise the CPU backend is used and the reason is
//! kept for the diagnostics overlay. A setting or a command-line option can force either backend.

use std::sync::{Mutex, OnceLock};

use ove_render::backend::{Backend, CpuBackend};
use ove_render::parity::{self, TOLERANCE};

use super::GpuBackend;
use crate::renderer::{GpuContext, GpuOptions};

/// Which backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The GPU when it passes the self-test, else the CPU.
    Auto,
    Cpu,
    /// The GPU without the self-test (any adapter, software included); refused when there is none.
    Gpu,
}

impl Choice {
    /// `auto`, `cpu` or `gpu`.
    pub fn parse(s: &str) -> Option<Choice> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Choice::Auto),
            "cpu" => Some(Choice::Cpu),
            "gpu" => Some(Choice::Gpu),
            _ => None,
        }
    }

    /// `OVE_BACKEND` when set to a valid name, else `fallback`.
    pub fn from_env_or(fallback: Choice) -> Choice {
        std::env::var("OVE_BACKEND").ok().and_then(|v| Choice::parse(&v)).unwrap_or(fallback)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Choice::Auto => "auto",
            Choice::Cpu => "cpu",
            Choice::Gpu => "gpu",
        }
    }
}

/// The chosen backend and why.
pub struct Selection {
    pub backend: Box<dyn Backend>,
    /// `"cpu"` or `"gpu"`.
    pub name: &'static str,
    /// The adapter, when the GPU is used.
    pub adapter: Option<String>,
    /// Why this backend: for the diagnostics overlay and command output.
    pub reason: String,
}

impl std::fmt::Debug for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selection")
            .field("name", &self.name)
            .field("adapter", &self.adapter)
            .field("reason", &self.reason)
            .finish()
    }
}

fn cpu(reason: String) -> Selection {
    Selection { backend: Box::new(CpuBackend::new()), name: "cpu", adapter: None, reason }
}

/// Self-test results by adapter name.
type Tested = Mutex<Vec<(String, Result<(), String>)>>;

/// Self-test results by adapter, so a process runs it once per adapter.
fn tested() -> &'static Tested {
    static T: OnceLock<Tested> = OnceLock::new();
    T.get_or_init(|| Mutex::new(Vec::new()))
}

/// Renders the self-test frames (layer drawing with blend modes, the basic grade, a LUT, a grade lattice and a
/// partial mix, at 192×108) on the GPU and the CPU and compares them with the parity tolerance. The error names the
/// failing case and its difference.
pub fn self_test(gpu: &mut GpuBackend) -> Result<(), String> {
    let (w, h) = (192, 108);
    let keep = |name: &str| {
        name.starts_with("draw")
            || ["blend multiply, transformed", "blend hue, transformed", "blend soft-light", "grade adjustments"]
                .contains(&name)
            || name.starts_with("grade lattice")
            || name == "lut cinematic"
            || name == "adjustment at half strength"
    };
    let cases: Vec<_> = parity::cases(w, h).into_iter().filter(|c| keep(&c.name)).collect();
    let rows = parity::check(&mut CpuBackend::new(), gpu, &cases, w, h, &TOLERANCE);
    if !gpu.healthy() {
        return Err("the GPU device was lost during the self-test".into());
    }
    match parity::failures(&rows) {
        f if f.is_empty() => Ok(()),
        f => Err(f.lines().next().unwrap_or_default().to_string()),
    }
}

/// Options for [`select_with`] (tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectOptions {
    /// Treat the self-test as failed (to check the fallback path).
    pub fail_self_test: bool,
}

/// Chooses a backend.
pub fn select(choice: Choice) -> Result<Selection, String> {
    select_with(choice, SelectOptions::default())
}

/// Chooses a backend. Only [`Choice::Gpu`] can fail: when no GPU adapter is found at all.
pub fn select_with(choice: Choice, opts: SelectOptions) -> Result<Selection, String> {
    match choice {
        Choice::Cpu => Ok(cpu("the CPU backend was chosen".into())),
        Choice::Gpu => {
            let ctx =
                GpuContext::new(GpuOptions { allow_software: true }).map_err(|e| format!("no usable GPU: {e}"))?;
            let gpu = GpuBackend::new(ctx);
            let adapter = gpu.adapter().to_string();
            Ok(Selection {
                backend: Box::new(gpu),
                name: "gpu",
                reason: format!("the GPU backend was chosen ({adapter})"),
                adapter: Some(adapter),
            })
        }
        Choice::Auto => {
            let ctx = match GpuContext::new(GpuOptions { allow_software: false }) {
                Ok(c) => c,
                Err(e) => return Ok(cpu(format!("no hardware GPU ({e}): using the CPU backend"))),
            };
            if ctx.adapter.get_info().device_type == wgpu::DeviceType::Cpu {
                return Ok(cpu("only a software GPU is available: using the CPU backend".into()));
            }
            let mut gpu = GpuBackend::new(ctx);
            let adapter = gpu.adapter().to_string();
            let result = if opts.fail_self_test {
                Err("forced failure".to_string())
            } else {
                let cached =
                    tested().lock().ok().and_then(|t| t.iter().find(|(a, _)| *a == adapter).map(|(_, r)| r.clone()));
                match cached {
                    Some(r) => r,
                    None => {
                        let r = self_test(&mut gpu);
                        if let Ok(mut t) = tested().lock() {
                            t.push((adapter.clone(), r.clone()));
                        }
                        r
                    }
                }
            };
            match result {
                Ok(()) => Ok(Selection {
                    backend: Box::new(gpu),
                    name: "gpu",
                    reason: format!("GPU self-test passed ({adapter})"),
                    adapter: Some(adapter),
                }),
                Err(e) => {
                    tracing::warn!(%adapter, "GPU self-test failed: {e}; using the CPU backend");
                    Ok(cpu(format!("GPU self-test failed on {adapter} ({e}): using the CPU backend")))
                }
            }
        }
    }
}
