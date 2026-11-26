# Design: extend-export-formats-and-queue

## Context

See proposal.md (Why). Today `ove-render::export` renders the timeline with the shared compositor, encodes H.264 + AAC
into MP4, writes a temporary file and renames it on success, and refuses offline media and pending items. One export
runs at a time, held in a process global in `exporting.rs`. Our FFmpeg build is LGPL-only and dynamically linked
(`openspec/config.yaml`), so GPL encoders (libx264, libx265) are not available. `prores_ks`, `hevc_videotoolbox`,
`prores_videotoolbox` and `hevc_mf` are already compiled but unused. Requirements: specs/export-formats,
specs/export-queue.

## Goals / Non-Goals

**Goals:**
- Every format a finishing house asks for on day one, within the licence rule.
- An export renders exactly the revision it was queued at, even while editing continues.
- The app, the CLI and agents see and control the same queue.

**Non-Goals:**
- HDR compositing (owned by `add-color-management-and-subject-mattes`); this change only encodes and tags HDR frames.
- Software HEVC or AV1 encoding.
- Distributed or cloud rendering.

## Decisions

### D1. One settings model, presets on top
`ExportSettings { container, video: Option<{codec, profile, bit_depth, quality, alpha}>, audio: Option<{codec,
sample_rate, bit_depth, bitrate}>, range: Option<[start, end]>, size: Height(u32) | MatchTimeline, fps }`. `video:
None` means audio-only. Presets ("YouTube 1080p", "ProRes 422 HQ master", "Web VP9", "Audio WAV 24-bit") are named
settings values in a JSON table, so the CLI, MCP and UI share them. A validation step rejects impossible combinations
(for example ProRes in MP4, alpha with H.264) with a message that names the conflicting fields.
Alternatives: keep flags per codec (grows without structure; agents cannot discover combinations); FFmpeg argument
passthrough (unsafe, untestable, breaks the licence gate).

### D2. Encoder selection per codec
Each codec has an ordered candidate list, filtered by what the FFmpeg build reports at run time:
- H.264: `h264_videotoolbox` / `h264_mf`, `h264_nvenc`, `h264_qsv`, `h264_amf` (fixed in `fix-render-and-edit-correctness`).
- HEVC 8/10-bit: `hevc_videotoolbox`, `hevc_nvenc`, `hevc_qsv`, `hevc_amf`, `hevc_mf`. No software fallback: the job
  fails with "no HEVC encoder on this computer" and suggests ProRes or H.264.
- ProRes: `prores_videotoolbox` on macOS, `prores_ks` elsewhere (and as macOS fallback). Labelled "ProRes (FFmpeg
  encoder)" when `prores_ks` is used.
- DNxHR: FFmpeg's built-in `dnxhd` encoder with DNxHR profiles.
- VP9: `libvpx-vp9` (BSD-3). AV1: `av1_nvenc`, `av1_qsv`, `av1_amf` only.
- Audio: `aac`, `pcm_s16le`, `pcm_s24le`, `flac`, `libopus` (BSD-3).
The chosen encoder and any fallback are reported in the job result. Alternatives: always software encoders (needs GPL
x264/x265 or nonfree, violates the licence rule); always hardware (fails on machines without a given encoder).

### D3. Alpha
ProRes 4444 with alpha and VP9 with alpha (WebM) carry the composite's alpha channel. The compositor already produces
straight-alpha RGBA; the export path premultiplies only where the codec expects it. The canvas background is
transparent instead of black when the settings request alpha. Other codecs reject `alpha: true` in validation.

### D4. HDR tags
When the compositor delivers HDR frames (signalled by the colour-management change), the exporter converts to 10-bit
(`p010` or `yuv422p10le`), sets BT.2020 primaries and matrix, PQ or HLG transfer, and writes mastering-display and
content-light side data. Until that change lands, an HDR request is refused with "HDR timelines are not available yet".
Alternative: up-convert the SDR composite to HLG at encode time — rejected, it labels SDR pictures as HDR.

### D5. Queue: job files plus a detached worker
Jobs are JSON files in the app data directory (`exports/<job-id>.json`), not in a bundle, because one queue serves all
projects. A job stores: project bundle path, pinned revision id, settings, output path, status (queued, running,
done, failed, cancelled), progress, chosen encoders, warnings, error, `requestedBy` (person, assistant, CLI, MCP
client name), timestamps and `worker_pid`. A worker process (`ove export --worker`) takes the oldest queued job,
renders the document reconstructed at the pinned revision, and updates the file at most 4 times per second. The app
and the CLI start a worker when none is alive. One encode runs at a time by default; a second runs only when both use
software encoders and the machine has at least 8 cores. Writes go to `<out>.partial` and are renamed on success, as
today.
Alternatives: in-memory queue in the app (lost on quit, invisible to the CLI); a queue per bundle (an export of project
A blocks nothing in project B but the UI needs a global view; per-bundle files would need a directory scan of every
project).

### D6. Revision pinning
Queuing records the current revision id. The worker opens the bundle read-only, reconstructs the document at that
revision through the existing restore path and renders it. Edits after queuing never change the output. If the
revision has been pruned before the job starts, the job fails with a clear message. Retention already keeps at least
2,000 revisions, and a queued job holds a pin that pruning respects.

### D7. Restart recovery
On start, the app and the CLI scan job files. A job in `running` whose `worker_pid` is not alive and whose file has
not changed for 30 seconds goes back to `queued` with `attempt += 1` and its partial output deleted. After 3 attempts
it fails. This reuses the dead-worker logic from generation jobs.

### D8. Save range as media and render in place
`--as-media` renders the range with the project's own frame size and rate into `media/rendered/<name>.<ext>`
(default ProRes 422 HQ on macOS, DNxHR HQ elsewhere; MP4 H.264 when the user picks "small"), then registers it with an
`AddMedia` op as bundle-owned media. `--replace` also removes or trims the covered items on the chosen tracks and
places one clip of the new media over the range, in the same revision. The rendered file exists before the revision is
committed; if the commit fails, the file is deleted. Undo restores the original items; the rendered media stays in the
library until removed.
Alternative: a nested-timeline "consolidate" (owned by `add-timelines-nesting-and-multicam`; a rendered file is what
people want for performance and hand-off).

### D9. Surfaces
- CLI: `ove export <project> [out]` keeps today's flags and adds the D1 fields. `--detach` returns the job id at once.
  `ove export queue` lists jobs, `ove export status <id> [--wait]`, `ove export cancel <id>`.
- MCP: `export_video` gains `range`, `wait` (default false), `preset`, and maps `format` and `quality` to the settings
  model. `job_status` and `cancel_job` accept export job ids. `save_range_as_media {project, start, end, replace,
  tracks}`.
- UI: the export dialog shows presets first and an Advanced section; the Exports panel lists jobs with progress,
  "Show in folder", "Cancel" and "Export again".

## Risks / Trade-offs

- [ProRes from `prores_ks` is not Apple-certified] → label it in the UI and job result; prefer VideoToolbox on macOS.
  Detection: an ffprobe check in tests confirms profile and pixel format; a QuickTime playback check in integration.
- [HEVC patents] → only OS and vendor hardware encoders, which carry their own licences. Detection: the licence check
  script refuses `libx265`.
- [Installer grows with libvpx and libopus] → measure in `sizes.txt` per platform; budget 8 MB extra. Detection: the
  packaging task fails if the installer exceeds 60 MB.
- [NVENC session limits on consumer GPUs] → one hardware encode at a time. Detection: an encoder-open failure is
  retried with the next candidate and reported.
- [A queued job outlives its bundle (moved or deleted)] → the worker fails the job with "project not found" before
  rendering. Detection: integration test moves a bundle while a job waits.
- [Render in place surprises people] → off by default, confirmation in the UI, one undo step.

## Migration Plan

`exporting.rs` callers switch to the queue client. Existing CLI flags keep their meaning. No document migration.

## Open Questions

- Wave order: this change proceeds in the master-plan order (wave 2) unless the owner reprioritises.
- SVT-AV1 is excluded pending a licence ruling (BSD-3-Clause-Clear plus the AOM patent licence); AV1 is offered only
  through hardware encoders.
- Default intermediate codec for "save range as media" on Windows: DNxHR HQ (proposed) or ProRes via `prores_ks`.
