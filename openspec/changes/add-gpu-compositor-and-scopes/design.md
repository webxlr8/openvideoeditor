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

## Results

### Group 1: numeric colour readout (CPU), measured 2026-11-27 on the Windows 11 x64 worker (CPU only, no GPU used)

- `ove-analyze::color_stats`: 9 unit tests on synthetic frames (grey ramp: mean 50.00 %, p2 1.96 %, p98 98.04 %,
  even 16-bin histogram; 75 % colour bars: saturation 6/7, six equal hue bins; warm and cool casts move CIELAB b* by
  more than 10; skin tone within 8° of the 123° line, red and yellow shifts on the expected sides; CIEDE2000 matches 6
  published reference pairs within 0.001).
- Deviation from D8: the suggestion is not built from `fit_match`. On the warm-shift loop, the Lab mean and
  standard-deviation fit of `fit_match` settled at a fixed point with mean ΔE00 4.2 (from 9.4) and did not improve
  with more rounds. `suggestedOps` now fits a line per channel through matching quantiles (5 % … 95 %, clipped values
  left out), composes it with the shot's current gain and offset wheels, and scales saturation to the reference's
  CIELAB chroma. Each step is capped (gain ±0.35, offset ±0.25, saturation ×0.75…1.33). `fit_match` is unchanged and
  still drives the app's Match colour action.
- Mock-agent loop (task 1.4, `crates/ove-cli/tests/color.rs`, H.264 test clips at 320×180, measured at 180 lines):
  mean ΔE00 per round 4.302 → 1.441 → 2.185 → 1.135. The first round, applied as `suggestedEdit` through the
  projection, is below 2. Later rounds, applied as `suggestedOps`, stay within ±1 of the goal (a small overshoot comes
  from composing per-channel wheels ahead of saturation).
- Repeat output is byte-identical (CLI test). Refusals exit 1: time past the end (names the duration), missing
  reference (names it), `--isolate` without `--item`, unknown item.
- Fixture note: the LGPL FFmpeg build on this machine has no still-image demuxer, so the tests use short H.264 clips
  made by the fixture FFmpeg instead of PNG stills.

### Group 2: backend trait and parity harness

- `ove_render::backend`: the `Backend` trait over image handles (`Tex`) with create, upload, read, release, size,
  `cpu` (run CPU code on an image), copy, draw, grade, lut, effect, transition mask, mix and opaque. Every operation
  has a default that uses `cpu` with the reference code, so a backend overrides only what it does natively (D2).
  `CpuBackend` keeps uploaded frames shared until written (no extra copy). The renderer drives the backend;
  `Renderer::new` keeps the CPU backend, `Renderer::with_backend` and `set_backend` take another. All 112 ove-render
  tests (unit, blend, effect and caption goldens, export safety, mixdown, motion, render) pass unchanged; the HDR
  fixture test was skipped (needs python3, `OVE_ALLOW_MISSING_MEDIA_TOOLS=1`).
- `ove_render::parity`: max channel difference, mean channel difference and CIEDE2000 p99 (ΔE computed only for
  pixels whose colour differs); one mosaic frame of 4 charts (75 % bars, grey ramp, hue and saturation sweep, alpha
  checker) and 3 procedural reference pictures (landscape, portrait stand-in, textured interior). The repository has
  no real camera frames, so the "real frames" are procedural stand-ins. 113 cases: 3 draws, 16 blend modes aligned
  and transformed, the basic grade, a grade lattice, 9 LUT presets, 41 effects, 23 visual transitions plus 2
  outgoing sides, and a half-strength adjustment.
- The effect goldens were already tolerance-based and shared by all three operating systems (wave 0,
  `tests/golden/effects-48x27.pam`, 1 step per channel); no Linux-only hashes were left to replace.
- Tasks 2.2 and 2.3 stay open until a CI run: the harness runs in the workspace test step on Linux, macOS and Windows
  and the new "GPU parity (software adapter)" step and nightly `gpu-parity` job are written, but no CI run was made
  from this worker (no push). Local results are under Group 3.

### Group 3: GPU compositor core (Windows 11 worker; adapters named per measurement)

- `crates/ove-gpu/src/compositor/`: `GpuBackend` implements the backend trait with wgpu compute shaders for layer
  drawing (inverse affine map, crop, rounded corners, bilinear sampling, opacity, all 16 blend modes and `add`), the
  basic grade (per-channel tables from `canvas::grade_tables`, then saturation and vibrance), LUTs and baked grade
  lattices (tetrahedral lookup), partial mixes and the final alpha pass. Effects, transition masks and captions run
  per layer on the CPU (readback, CPU code, upload) until they are ported (D2).
- Deviations from D1 and 3.1, with reasons: images are `rgba8unorm` storage textures and every shader rounds to 8
  bits where the CPU reference does (the reference is 8-bit, so this keeps parity at 1 step; 16-bit float
  intermediates come with add-color-management-and-subject-mattes). Lattices are storage buffers read with the same
  tetrahedral arithmetic as the CPU, not 3D textures (hardware filtering is trilinear and less exact). Which pixels a
  hard layer edge covers is decided on the CPU per row with the reference's f64 arithmetic and passed as spans:
  computed in f32 on the GPU, single edge pixels flipped (one case measured max 184/255 at one pixel before this).
- Parity (`cargo test -p ove-gpu --test parity`, 113 cases at 1920×1080, every case within tolerance):
  - NVIDIA GeForce RTX 4070 (Dx12, DiscreteGpu, driver 32.0.16.1088): worst max 1/255, worst mean 0.000/255,
    CIEDE2000 p99 0.000 everywhere; CPU 7.5 s, GPU 5.2 s for all cases including uploads and readbacks.
  - Microsoft Basic Render Driver (WARP; Dx12, Cpu, driver 10.0.22621.5160): worst max 1/255; GPU 113 s.
  - 97 native GPU passes and 57 CPU fallbacks per run (effects, masks).
- Wrong shader (task 2.3 check): the difference blend scaled by 0.9 fails exactly "blend difference" (max 25/255,
  mean 3.222/255, CIEDE2000 p99 5.347) and "blend difference, transformed" on both the RTX 4070 and WARP.
- Whole project (`render_parity`, 360p, 53 frames: one clip per effect for all 41 effects, 8 blend modes over
  transformed, cropped, rounded clips, grade with wheels and a LUT, a wipe join, a push, a shader item, text):
  RTX 4070 worst max 1/255, worst mean 0.010/255.
- Selection (D9): `select(auto)` opens a hardware adapter (software adapters are not used automatically), runs the
  self-test (draws, 3 blend modes, grade, lattice, LUT, mix at 192×108) once per adapter and process, and falls back
  to the CPU with the reason. RTX 4070: "GPU self-test passed". A forced self-test failure gives the CPU backend and
  "GPU self-test failed …". A device loss during rendering (simulated with `LossHandle::trigger`) switches the
  renderer to the CPU backend, logs it, and the next frame equals the CPU frame byte for byte.
- `ove render` (alias of `export`), `look` and `color inspect` take `--backend auto|cpu|gpu` (also `OVE_BACKEND`)
  and report `"backend": {"name", "adapter", "reason"}`; an unknown name exits 1 naming the valid ones. The app keeps
  the choice in `compositing.json` (Settings → Rendering, on by default), runs the self-test at startup in the
  background, uses it for the timeline preview, Match colour and export, and the diagnostics overlay shows
  "timeline compositor (GPU, <adapter>; N ms/frame)" or "(CPU — <reason>; …)". UI test: the toggle switches the
  status between "Active: GPU (…)" and "Active: CPU — the GPU is turned off in Settings."
- Not measured here: the built app on macOS and Windows (task 7.1).

### Group 4: effects, transitions and alpha

- 4.1: the per-pixel effect function returns RGBA (`PixelFn::Rgba`) or RGB (`PixelFn::Rgb`, input alpha kept). All
  28 colour and distortion effects keep the alpha of a semi-transparent layer exactly; the 13 background generators
  fill their area as before (unit test `existing_effects_keep_input_alpha`). The effect goldens are unchanged.
- 4.2: keyframe tracks with the property `effects.<effect id>.<parameter>` animate a numeric effect parameter with the
  shared evaluator (`eval::track_value`, the one transform keyframes use). Zoom blur intensity keyframed 0 → 20 over
  1 s gives 10 at 0.5 s and the same pixels as a static 10. `ove source apply` refuses a track on a parameter the
  effect does not have (`keyframe.effect.param`, "effect zoom-blur ("b") has no numeric parameter "radius" …") or on
  an effect id the item does not have (`keyframe.effect.exists`). ove-core keeps the table of numeric parameters
  (`effect_params.rs`), and an ove-render test keeps it equal to the effect contract. The JSON schema and
  `timeline.d.ts` accept the new property form (`effects.${string}.${string}`).
- 4.3 (open; Option A, approved by the CTO and CEO: port by measured cost, every port passes parity on the RTX 4070
  and WARP before it lands). Per-frame cost at 1920×1080 (`cargo test -p ove-gpu --test costs -- --ignored`,
  NVIDIA GeForce RTX 4070, median of 5; "GPU backend" is the native shader, or the CPU fallback with readback and
  upload):
  - Native now: zoom-blur 68.9 ms on the CPU → 3.4 ms; tilt-shift 56.7 → 3.0 ms; aurora 70.2 → 0.8 ms; transition
    masks wipe-left 3.6 → 0.6 ms, radial-wipe 9.6 → 1.0 ms, ink-bleed 17.6 → 0.8 ms, mask-glitch 8.3 → 0.8 ms.
    Parity at 1080p: worst max 1/255 on the RTX 4070 and on WARP for each.
  - On the CPU fallback: 38 of the 41 effects (duotone, tritone, gradient-map, posterize, solarize, halftone, dither,
    engraving, watercolor, pixelate, chromatic-aberration, glitch, vhs, crt-screen, light-leak, lens-flare, bulge,
    spherize, lens-distortion, twirl, wave-distortion, kaleidoscope, mirror, polar-coordinates, fluted-glass,
    glass-tiles, mesh-gradient, flowing-gradient, plasma, swirl, marble, godrays, sunburst, spiral, conic-gradient,
    voronoi, stripes, studio-background), and so the glitch, chromatic-aberration and light-leak post effects of
    transitions. Costliest on the fallback: watercolor 80.9 ms, flowing-gradient 43.8 ms.
  - Watercolor was ported and failed parity: its pigment-pool step `round(x·6)` lands on the other side of a step on
    a few pixels (max 27/255, mean 0.000/255), so it stays on the CPU. Hard quantisation steps like this need exact
    arithmetic shared by both sides before they can move.
  - The effect noise uses the existing integer hash (`util::hash21`), ported bit for bit, rather than a new PCG
    hash: it is already an integer hash on both sides, and changing it would change every noise effect's look.
