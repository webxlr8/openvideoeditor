# Design: add-gpu-compositor-and-scopes

## Context

See proposal.md (Why). Today `crates/ove-render` composites straight-alpha RGBA8 on the CPU (`canvas.rs`), bakes the
grade into one 33³ lattice (`color.rs`), and runs 41 effects as per-pixel functions that return RGB only
(`effects/mod.rs`). wgpu is used only to convert YUV to RGBA16F and present the viewer; `crates/ove-gpu/src/cpu.rs` is
already a CPU oracle for that step. Timeline preview is CPU-composited at 540p, 30 fps
(`apps/desktop/src-tauri/src/timeline.rs`). `color_scopes` builds a new renderer at 180p for every call
(`apps/desktop/src-tauri/src/fx.rs`). Requirements: specs/gpu-compositor, specs/color-scopes.

## Goals / Non-Goals

**Goals:**
- One compositor for preview, `look` and export, with a CPU path that stays the reference for tests.
- 1080p30 preview with several layers and effects on the reference machines.
- Numbers an agent can loop on to match shots, measured on the same pixels that get exported.

**Non-Goals:**
- A float or log working space (add-color-management-and-subject-mattes). This change stays display-referred 8-bit
  equivalent in output, with 16-bit float intermediates on the GPU.
- New looks or effects beyond `chroma-key`.

## Decisions

### D1. Backend trait in ove-render, GPU code in ove-gpu
`ove-render` defines a backend trait with four operations: draw a layer, apply the grade lattice, apply an effect, run
a transition. The CPU backend wraps today's code. The GPU backend lives in `crates/ove-gpu/src/compositor/` and uses
WGSL compute and render passes on rgba16f textures. The evaluator (`eval.rs`) and layer ordering stay shared, so both
backends get identical layer lists. Alternatives: a separate GPU renderer with its own evaluation (two sources of
truth, drift); GPU only with no CPU path (no oracle on CI runners without a GPU, and no fallback on broken drivers).

### D2. Port order by cost and use
1. Layer draw, all 16 blend modes, grade lattice (3D texture, tetrahedral lookup) and LUT stages.
2. Separable blurs (zoom blur, tilt-shift, disc blur, glow).
3. Transitions.
4. The remaining per-pixel effects. They are pure functions of position and parameters, so the port is mechanical.
5. Distortion effects.
Any effect without a GPU version yet runs on the CPU for that layer: the GPU backend reads the layer back, runs the CPU
effect, and uploads it. This keeps every project renderable during the port. Alternative: port everything before
switching on (a long period with no user value).

### D3. Parity by tolerance, not by hash
Floating-point results differ between WGSL and Rust. Noise effects use a shared integer hash (32-bit PCG) on both
sides, so their patterns match. Parity is measured per effect and per transition on test charts and three real frames
at 1080p: max channel difference ≤ 2/255, mean ≤ 0.25/255, CIEDE2000 p99 ≤ 1.0. Alternative: bit-exact output (not
reachable across GPU vendors).

### D4. CI on software GPUs, real GPUs nightly
Parity tests run on WARP (Windows) and lavapipe (Linux) in CI. A nightly job runs them on real Apple and Windows GPUs
and records timings. The existing Linux-only effect golden hashes are replaced by the tolerance harness, which runs on
all three operating systems. Alternative: GPU tests only on developer machines (regressions found late).

### D5. RGBA effect contract and keyframable parameters
The per-pixel effect function returns RGBA instead of RGB, so keys and mattes can write alpha. Existing effects return
their input alpha. Effect parameters accept keyframe tracks with the same evaluator as transform keyframes. A parameter
with no keyframes behaves exactly as today. Alternative: a separate "matte" effect class (two contracts to maintain).

### D6. Chroma key in the CbCr plane
Key distance is measured in the CbCr plane and scaled by luma, so dark and desaturated screens still key. Spill
suppression limits the key channel toward a mix of the other two channels and restores luma. Matte black and white
levels, choke and edge blur shape the matte. The key runs in source pixel space before the transform, on chroma
upsampled at decode. The sampler averages a clicked point or a dragged rectangle of the source before grading.
Alternative: hue distance only (fails on dark or desaturated screens and gives no matte control).

### D7. Scopes from the playback frame
Scopes bin the frame that playback already composited. They do not render again. The GPU path uses a compute pass with
atomic counters; the CPU path bins on a worker thread. Scope data goes to the web view as u8 density arrays (about
200 KB) at up to 15 Hz and is drawn on a canvas. Alternative: send RGBA frames to the UI and bin in JavaScript (about
8 MB per update at 1080p).

### D8. inspect_color through the real compositor
`ove-analyze::color_stats` measures the composited frame at the exact time, or only the named item with `--isolate`.
Balance uses CIELAB a* and b*. Zones are shadows, midtones and highlights by luma. With a reference (item or media at a
time) it reports per-zone ΔE00 and a `suggestedOps` patch for `colorGrade.wheels` and `saturation`, built from the
existing `fit_match` and capped in strength. Output is deterministic JSON. It is CPU-only, so it ships before the GPU
work. Alternative: decode the source directly and apply only the clip's effects (numbers would not match export).

### D9. Backend selection and fallback
The GPU backend is the default when an adapter is found and a startup self-test frame matches the CPU reference within
tolerance. Otherwise the CPU backend is used and the diagnostics overlay says why. Settings has a toggle.
`ove render --backend` overrides. A device loss during preview falls back to the CPU and logs it. Export never changes
backend in the middle of a job.

## Risks / Trade-offs

- [Driver bugs give wrong pixels on some GPUs] → startup self-test against the CPU reference; automatic CPU fallback;
  detected by the self-test result in the diagnostics overlay and the crash report.
- [Readback for not-yet-ported effects is slow] → port order by measured cost; detected by per-frame timing in the
  diagnostics overlay and the nightly benchmark.
- [Agents overfit to mean values in inspect_color] → report zones and percentiles; cap suggested op strength; detected
  by the mock-agent loop test (task 1.4).
- [IPC bandwidth for live scopes] → binned u8 arrays at 15 Hz; detected by measured IPC bytes per second in the
  diagnostics overlay.

## Migration Plan

No document change. Effects without keyframes render as before. Projects that used blend modes other than the six old
ones change appearance once, which is the bug fix in fix-render-and-edit-correctness.

## Open Questions

- Waves proceed in the master-plan order (default until the owner decides otherwise).
- DirectML is not used here; wgpu on D3D12 and Metal only.
- Reference machines for the 1080p30 gate: M1 (8-core GPU) and a laptop with Intel Iris Xe, unless the owner names
  others.
