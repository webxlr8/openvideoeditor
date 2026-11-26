# Proposal: add-gpu-compositor-and-scopes

## Why

The compositor runs only on the CPU. It composites at 540p and 30 fps for preview. Export at 4K is slow, and effects
cannot write alpha, so a chroma key is not possible. The only colour scope is a 180p histogram that opens new decoders
on every call. Agents can see frames, but they cannot read numbers that tell them how far two shots are apart, so shot
matching is guesswork. This change moves compositing to the GPU and keeps the CPU path as the reference. It adds real
scopes for people and numeric colour readouts for agents.

## What Changes

- A GPU compositing backend for layer drawing (transform, crop, corner radius, opacity), all 16 blend modes, the grade
  lattice with LUT stages, the 24 transitions and the 41 effects. The CPU path stays as the reference. Both paths must
  agree within stated tolerances.
- One compositor for preview, agent `look` frames and export. Preview resolution goes up to 1080p when the GPU backend
  is active.
- The effect contract changes so that an effect can write alpha. Effect parameters become keyframable, which gives
  animated blur and similar effects.
- A new `chroma-key` effect. It has key colour, tolerance, softness, spill suppression, choke, matte black and white
  levels, edge blur and a matte view. An area sampler in the viewer picks the key colour from the source.
- Waveform, RGB parade and vectorscope scopes in the colour panel, computed from the frame that playback already
  rendered.
- `inspect_color` for agents. It is frame-exact and runs through the real compositor. It reports luma percentiles,
  clipping, zone means, colour balance, a saturation-weighted hue histogram and skin-line deviation. With a reference it
  reports the gap in perceptual units and a ready patch of suggested grade operations. This item ships first, on the
  CPU, before any GPU work.
- `ove look --scope waveform|parade|vectorscope` writes scope images. `ove render --backend cpu|gpu` selects the
  backend. Settings gets a GPU toggle. The diagnostics overlay names the active backend.

Supersedes add-effects-and-color tasks 1.3 (GPU path) and 1.5 (waveform and vectorscope scopes).

Out of scope: colour management, log decoding and HDR output (add-color-management-and-subject-mattes); subject
mattes and `region` (same change); on-canvas handles and guides (add-viewer-interaction-and-text-fills); new effects
other than `chroma-key`; GPU text rasterisation (text, captions and code items stay CPU-rasterised and upload as
textures).

References: docs/spec/presets.md (effects, transitions, layouts, LUTs), docs/spec/project-schema-v1.md (`blendMode`,
`effects`, `colorGrade`, `keyframeTracks`), docs/spec/timeline.d.ts, docs/spec/cli.md (`look`), docs/spec/mcp-tools.md.

## Capabilities

### New Capabilities
- `gpu-compositor`: GPU compositing with CPU parity, alpha-writing and keyframable effects, the chroma key and its
  sampler, backend selection and preview resolution.
- `color-scopes`: waveform, parade and vectorscope scopes for people, scope images for agents, and the numeric
  `inspect_color` readout with reference matching.

### Modified Capabilities
- None. The effect set and parameter ranges in docs/spec/presets.md stay the same.

## Impact

- New module `crates/ove-gpu/src/compositor/`. `crates/ove-render` gains a backend trait over draw, grade, effect and
  transition. `crates/ove-render/src/effects/mod.rs` changes its pixel contract to RGBA. New
  `crates/ove-render/src/effects/key.rs` and `crates/ove-render/src/scopes.rs`. New `ove-analyze::color_stats`.
- CLI and MCP: `ove color inspect`, MCP `inspect_color`, `look --scope`, `render --backend`. Tool registration goes
  through the registry from unify-agent-tool-layer when it exists; until then through `crates/ove-cli/src/mcp.rs`.
- UI: scopes canvas and chroma-key sampler in `apps/desktop/ui/src/FxControls.tsx` and the viewer.
- Depends on fix-render-and-edit-correctness (CPU mapping of all 16 blend modes, which the GPU path must match).
- Feeds add-color-management-and-subject-mattes (it needs the float GPU pipeline and the RGBA effect contract) and
  add-viewer-interaction-and-text-fills (difference blend for inverted text).
- No new dependencies: wgpu and naga are already in Cargo.lock.
