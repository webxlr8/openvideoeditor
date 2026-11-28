# Proposal: extend-export-formats-and-queue

## Why

Export writes one format: H.264 + AAC in MP4, at a fixed height, one job at a time. Production houses need
intermediate codecs (ProRes, DNxHR), HEVC, alpha for overlays, web formats, audio-only stems and a range of the
timeline rendered back into the project. A long export blocks every other export, and a restart loses it. Agents can
start an export but cannot queue, watch or cancel it. The MCP `export_video` contract in docs/spec/mcp-tools.md already
promises `format` and `quality`, which we do not honour.

## What Changes

- Export settings model: container, video codec, profile, bit depth, quality, audio codec, sample rate and bit depth,
  a time range, and size as a height preset or "match timeline".
- Video codecs: H.264, HEVC 8-bit and 10-bit (hardware encoders only), ProRes 422 Proxy / LT / 422 / HQ and 4444 with
  alpha, DNxHR (LB, SQ, HQ, HQX, 444), VP9 in WebM or MKV, and AV1 through hardware encoders when present.
- Audio-only export: AAC in `.m4a`, WAV 16-bit or 24-bit, FLAC.
- Containers: MP4, MOV, MKV, WebM, M4A, WAV, FLAC.
- HDR encoder support: HEVC Main10 and ProRes 10/12-bit with BT.2020 PQ or HLG colour tags and mastering metadata, used
  when the timeline produces HDR frames (the HDR compositor mode is owned by `add-color-management-and-subject-mattes`).
- Export queue: jobs persisted on disk, run by a detached worker, survive an app restart, each pinned to the project
  revision it was queued at, visible to the app, the CLI and MCP.
- Save range as media: render a timeline range into the project's media; optional "render in place" replaces the
  covered items with the rendered clip in one revision.
- CLI: `ove export` gains `--format`, `--video-codec`, `--profile`, `--bit-depth`, `--audio-codec`, `--range`,
  `--size match`, `--detach`, `--as-media`, `--replace`; `ove export queue|status|cancel`.
- MCP: `export_video` honours `format`, `quality`, `range`, `wait`; `job_status` and `cancel_job` cover export jobs;
  `save_range_as_media`.
- UI: export dialog with presets and advanced settings; Exports panel listing queued, running and finished jobs.

Out of scope: the HDR working space and tone mapping in the compositor (`add-color-management-and-subject-mattes`);
interchange formats and project packaging (`improve-interchange-and-packaging`); AI generation; loudness normalisation
and the true-peak limiter (`add-audio-mixing-and-monitoring`); the encoder-chain fix for the current H.264 path
(`fix-render-and-edit-correctness`); cloud export (`--cloud`); GIF export; software HEVC or AV1 encoders.

References: docs/spec/cli.md (`export`), docs/spec/mcp-tools.md (`export_video`, `job_status`, `cancel_job`),
docs/cli/export.md, docs/spec/document-model.md (media registry, ownership).

## Capabilities

### New Capabilities
- `export-formats`: the export settings model, video, audio and container choices, alpha, HDR tags, audio-only
  export, and save-range-as-media with render in place.
- `export-queue`: persisted export jobs, revision pinning, detached worker, progress, cancel, restart recovery, and
  the CLI, MCP and UI surfaces for the queue.

### Modified Capabilities
- None.

## Impact

- `crates/ove-render/src/export.rs` (settings model, encoder selection per codec), new `crates/ove-render/src/queue.rs`.
- `tools/ffmpeg/build.sh`: enable `hevc_*`, `prores_*`, `dnxhd`, `libvpx`, `libopus`, `av1_*` hardware encoders and the
  `mov`, `matroska`, `webm`, `ipod`, `wav`, `flac` muxers; LGPL check stays the gate; installer size measured.
- `apps/desktop/src-tauri/src/exporting.rs` (replaced by queue client), export dialog and new Exports panel in
  `apps/desktop/ui`.
- `crates/ove-cli` (`export` flags, `export queue|status|cancel`), `crates/ove-cli/src/mcp.rs` (tool handlers move to
  the registry from `unify-agent-tool-layer` when it lands).
- Depends on: `fix-render-and-edit-correctness` (working encoder chain). Used by: `extend-ai-generation-actions`
  (range render for preprocessing), `improve-interchange-and-packaging` (trim-to-used transcodes),
  `add-color-management-and-subject-mattes` (HDR output).
