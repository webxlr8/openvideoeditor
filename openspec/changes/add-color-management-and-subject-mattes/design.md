# Design: add-color-management-and-subject-mattes

## Context

See proposal.md (Why). `ove-media` detects matrix, transfer (PQ, HLG) and BT.2020 primaries (`convert.rs`), then
tone-maps HDR to SDR BT.709 at decode. The viewer has a PQ/HLG tone-mapper with a CPU oracle (`ove-gpu` shaders and
`cpu.rs`). The grade lattice works on display values from 0 to 1 (`color.rs`). Export tags BT.709. The `region` field
and caption `behind-subject` exist in the schema but fall back with a label. Requirements: specs/color-management,
specs/subject-mattes.

## Goals / Non-Goals

**Goals:**
- Correct colour from log and HDR sources through to SDR or HDR output, the same on both operating systems.
- Old projects render exactly as before.
- Mattes good enough for talking heads and social video, computed locally.

**Non-Goals:**
- A configurable colour engine with user configs.
- Professional rotoscoping.

## Decisions

### D1. Explicit output space
The output space is a project setting. It is never inferred from a source clip. Mixed timelines therefore always end
in one known space. Alternative: inherit tags from the top visible clip (output space would change from frame to
frame on mixed timelines).

### D2. Own transforms from published formulas
`colorspace.rs` implements the transfer curves and primaries matrices from vendor and standards documents. Each curve
has tests against published tables. Alternative: OpenColorIO (BSD-3, but a large C++ build, and we need a fixed small
set of transforms).

### D3. Grade in the working encoding, composite in linear light
Per layer: input transform to the working space, grade lattice in the working space's encoding (log-like for
`acescct` and `dwg-intermediate`), convert to linear light, composite, then output transform with tone mapping. The
lattice moves to 65³ in half float where the measured error requires it (target error below 0.1%). With the `rec709`
working space, the pipeline is the identity of today's path. Alternative: grade in linear light (wheels and curves feel
wrong to colourists in linear).

### D4. Float on the GPU, CPU stays the oracle
The GPU path uses rgba16f end to end. The CPU path moves to f32 and stays the parity reference at reduced resolution.
The 8-bit CPU path cannot carry log or HDR, so this change requires the GPU compositor for full-resolution work.

### D5. HDR output and preview
HDR export writes 10-bit frames with PQ or HLG transfer, BT.2020 primaries, and mastering display and content light
metadata. SDR graphics (text, captions, code items) are placed at 203 nits (BT.2408). Preview uses an HDR swapchain
where available (EDR on macOS, HDR10 or scRGB on Windows). Otherwise the preview tone-maps the HDR timeline to SDR with
the existing tone-mapper, while export stays HDR. Alternative: block HDR export when the display is SDR (users edit on
SDR laptops and deliver HDR).

### D6. Mattes as a background job with a sidecar cache
`matte.rs` runs a segmentation model at about 512 px per frame in the inference worker, applies a temporal moving
average to reduce flicker, and writes a lossless grey matte sidecar keyed by `contentId` in the bundle cache. Jobs run
through the background scheduler at low priority, and pause during playback and export. The renderer upsamples the
matte with a guided filter that follows the full-resolution frame's edges. Alternative: compute mattes live during
preview (too slow on the CPU fallback, and results would differ between preview and export).

### D7. Model choice and runtime
Candidate models: MODNet (Apache-2.0), BiRefNet (MIT), MediaPipe Selfie Segmentation (Apache-2.0). Excluded: RVM
(GPL-3.0) and RMBG 1.4 and 2.0 (non-commercial weights). The weight licence is checked separately from the code licence
before the model is pinned. The model and the runtime come from add-model-runtime: downloaded on first use, SHA-pinned,
with consent above 50 MB. Execution uses the CoreML provider on macOS and the CPU on Windows; DirectML stays off until
its licence is ruled on.

### D8. What mattes drive
- Effect `region: subject|background` multiplies the effect's result by the matte or its inverse.
- Captions `behind-subject` render the caption between the background and the subject (matte-cut).
- Footage-filled text can take the subject as its fill source.
- `ove color inspect --region subject` measures only matte pixels.
When a matte is missing or still computing, these features fall back to the whole frame and `verify` reports a
warning naming the item. Export refuses while a needed matte is still computing, unless `--allow-missing-mattes` is
given.

## Risks / Trade-offs

- [HDR swapchains are immature in wgpu on some platforms] → SDR tone-mapped preview fallback; detected by a startup
  capability check shown in the diagnostics overlay.
- [Old projects look different] → `rec709` default is the identity of today's path; detected by golden frames of old
  projects in CI.
- [Matte flicker] → temporal smoothing plus the guided filter; detected by a flicker metric (mean change in static
  regions) in tests.
- [Installer size] → models and runtime download on demand; detected by the installer size check in CI.
- [Model licence changes upstream] → pin by SHA and record the licence text in the change Results; detected by review
  at each model bump.

## Migration Plan

Projects without `project.color` use `rec709` working and output spaces, which match today's output. Media without
`colorSpace` use the detected tags. No document migration is needed.

## Open Questions

- Waves proceed in the master-plan order (default until the owner decides otherwise).
- Models and the inference runtime download on first use with explicit consent above 50 MB (default).
- DirectML is off by default; the CPU is used on Windows until the licence is ruled on.
- Matte model weight licences are verified before pinning; RVM and RMBG are excluded.
- HDR test displays: a macOS XDR display and a Windows HDR10 monitor; the owner confirms access.
