# Proposal: fix-render-and-edit-correctness

## Why

A review of the editor against its own schema and specs found defects that give wrong output without any warning:
layers with 10 of the 16 blend modes render as Normal, splitting a sped-up or reversed clip shows the wrong source
frames, trimming or splitting a clip moves its keyframes and caption words to the wrong time, the shipping FFmpeg build
lacks most of the H.264 encoders the exporter tries, and a paid generation is thrown away when the app restarts while it
runs. These are fixed first, before new features build on them.

## What Changes

- Render all 16 schema blend modes on the CPU compositor with the standard compositing formulas, instead of mapping 10
  of them to Normal and `color-dodge` to Add. The legacy `add` value keeps working.
- Split: speed- and reverse-aware source mapping (`trimStart`, `trimEnd`); keyframes and caption segments (with word
  timings) cut at the split point and rebased; start-edge fields (fade in, audio fade in, transition in) stay on the
  left part and end-edge fields on the right part; right-part ids that cannot collide.
- Trim: moving the start edge keeps keyframes and caption segments at the same timeline times; reverse playback moves
  the correct source edge.
- Export encoders: enable the hardware H.264/HEVC encoders for NVIDIA, Intel and AMD, and OpenH264 as the software
  fallback, in the LGPL FFmpeg build, so a failing platform encoder no longer drops to MPEG-4 Part 2.
- Generation: a job whose worker stopped while the provider still runs it resumes and fills its placeholder instead
  of failing; jobs that stopped making progress still fail.
- Honest audio labels ("Lower music to 30 %", "Reset volumes to 100 %") are kept as a requirement until real ducking
  and loudness normalisation land (add-audio-mixing-and-monitoring); they already shipped in commit 8b5eb09.
- Bookkeeping: add-ai-generation task 1.2 is marked open again for its MCP part, which moves to
  unify-agent-tool-layer; effect golden tests also run on macOS and Windows.

Out of scope: GPU blending (add-gpu-compositor-and-scopes), the edit planners crate that will own split and trim
(add-edit-planners-and-links), real ducking and loudness (add-audio-mixing-and-monitoring), generation over MCP
(unify-agent-tool-layer), new export codecs (extend-export-formats-and-queue).

References: docs/spec/timeline.d.ts (`blendMode`), docs/spec/document-model.md (item timing fields, keyframes, caption
segments), docs/spec/cli.md (`export`), docs/spec/presets.md (blend modes).

## Capabilities

### New Capabilities
- `render-correctness`: blend modes, split and trim timing, export encoder availability, generation job resume and
  audio button labels behave as the schema and labels state.

### Modified Capabilities
- None.

## Impact

- `ove-render` (blend modes in `canvas.rs` and `eval.rs`; the renderer's easing now comes from `ove-core`).
- New module `ove-core::retime` (split and start-edge rebase), used by the desktop app's split and trim commands;
  add-edit-planners-and-links moves callers to its planners later.
- `tools/ffmpeg/build.sh` (encoder flags and their headers/libraries), `tools/ffmpeg/check-license.sh`.
- `ove-generate` (job record gains the provider's remote handle; resume path), desktop and CLI reaping callers.
