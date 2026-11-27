//! The compositing backend for tools that render frames (`look`, `inspect_color`, `export_video`): the `backend`
//! argument (`auto`, `cpu` or `gpu`), else `OVE_BACKEND`, else `auto` (the GPU when it passes its self-test).

#![cfg(feature = "media")]

use ove_gpu::compositor::{Choice, Selection};
use serde_json::{Value, json};

use crate::fail::Fail;
use crate::spec::Args;

/// The argument's description, shared by the tools that take it.
pub const ABOUT: &str =
    "Compositing backend: auto (default; the GPU when its startup self-test matches the CPU reference), cpu or gpu";

/// Chooses the backend for this call.
pub fn choose(a: &Args) -> Result<Selection, Fail> {
    let choice = match a.str("backend") {
        Some(s) => Choice::parse(s)
            .ok_or_else(|| Fail::new("argument.invalid", format!("backend is auto, cpu or gpu, not {s}")))?,
        None => Choice::from_env_or(Choice::Auto),
    };
    ove_gpu::compositor::select(choice).map_err(|e| Fail::new("gpu.unavailable", e))
}

/// The process's backend provider for renders that do not choose one themselves (queued exports): the requested
/// choice, else `OVE_BACKEND`, else `auto`. Installed by `ove` at start.
pub fn install_provider() {
    ove_render::backend::set_provider(|choice| {
        let c = match choice {
            Some(s) => Choice::parse(s).ok_or_else(|| format!("backend is auto, cpu or gpu, not {s}"))?,
            None => Choice::from_env_or(Choice::Auto),
        };
        let s = ove_gpu::compositor::select(c)?;
        Ok((s.backend, format!("{}: {}", s.name, s.reason)))
    });
}

/// What a receipt says about the backend.
pub fn json(s: &Selection) -> Value {
    json!({"name": s.name, "adapter": s.adapter, "reason": s.reason})
}
