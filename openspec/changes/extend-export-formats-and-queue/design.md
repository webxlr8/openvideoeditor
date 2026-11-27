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

## Results

Measured on the Windows 11 x64 development machine (no NVIDIA, AMD or Intel encoder exposed to FFmpeg) with the
LGPL FFmpeg 9.0.2 build in `H:/Development/OpenvideoEditor/tools/ffmpeg/out/windows-x64`, debug test builds.

### Group 1 (FFmpeg build)

- `tools/ffmpeg/build.sh` now enables the `dnxhd`, `libvpx_vp9`, `libopus`, `flac` and `pcm_s24le` encoders, the
  `ipod`, `matroska`, `webm`, `wav` and `flac` muxers, and on native Windows NVENC, AMF and Quick Sync (`h264_*`,
  `hevc_*`, `av1_*`) through MIT-licensed headers. `check-license.sh` also refuses `--enable-libsvtav1`. The existing
  build passes the updated check. The rebuilt libraries and installer size could not be produced here (no MSYS2,
  vcpkg libvpx/opus or macOS machine): task 1.1 stays open.
- This build's encoder inventory (`ove doctor`, `Inventory::current()`): `aac`, `h264_mf`, `hevc_mf`, `mjpeg`,
  `mpeg4`, `pcm_f32le`, `pcm_s16le`, `png`, `prores_ks`, `rawvideo`, `wrapped_avframe`, …; muxers `mp4`, `mov`,
  `framemd5`, `md5`, `null`, `s16le`, `f32le`.
- `ove doctor` now reports each codec's working encoders by writing a one-frame file with each candidate. On this
  machine: H.264 `h264_mf`, `mpeg4`; ProRes `prores_ks`; HEVC none (`hevc_mf` opens but refuses the test picture;
  NVENC, Quick Sync, AMF not in this build); DNxHR, VP9, AV1 none (not in this build). The macOS reference machine is
  not available here: task 1.2 stays open.

### Group 2 (settings model and encoders)

- Codec matrix on a 10 s 640×360 timeline (test pattern, tone, title), each export probed with ffprobe and compared
  with the H.264 export (`h264_mf`, very high quality) by SSIM on luma (all-plane SSIM in brackets; 4:2:2 and 4:4:4
  files lose chroma detail to the comparison's own 4:2:0 subsampling):

  | Export | Encoder | Probe (codec, profile, pix_fmt, colour) | SSIM luma (all) |
  |---|---|---|---|
  | ProRes 422 Proxy | prores_ks | prores Proxy yuv422p10le bt709 | 0.9839 (0.9848) |
  | ProRes 422 LT | prores_ks | prores LT yuv422p10le bt709 | 0.9948 (0.9939) |
  | ProRes 422 | prores_ks | prores Standard yuv422p10le bt709 | 0.9955 (0.9944) |
  | ProRes 422 HQ | prores_ks | prores HQ yuv422p10le bt709 | 0.9958 (0.9945) |
  | ProRes 4444 | prores_ks | prores 4444 yuv444p12le (decoder) bt709 | 0.9958 (0.9898) |

  ProRes 422 Proxy is the offline-editing profile at about a fifth of HQ's data rate; its floor in the test is 0.98,
  every other profile meets 0.99.
- Not verifiable on this machine, refused cleanly with the expected message, verification left open: HEVC 8-bit
  (`hevc_mf` opens but refuses the test picture: "Generic error in an external library"), HEVC 10-bit (`hevc_mf` takes
  no 10-bit 4:2:0 format), DNxHR (no `dnxhd` in this build), VP9 and VP9 alpha (no `libvpx-vp9`, no WebM muxer), AV1
  (no hardware AV1 encoder), FLAC (no `flac` encoder or muxer).
- Hardware encoders are tried with one test picture before use: an encoder that opens but cannot encode is reported
  in `encoderFallbacks` and the next candidate is used.
- Alpha: ProRes 4444 with alpha of a text-only timeline: 225,469 pixels outside the text have alpha 0 and 2,939
  pixels inside it have alpha 255 (decoded with the reference ffmpeg).
- HDR: an HDR request on the SDR timeline is refused with "HDR timelines are not available yet…" and writes nothing.
  A synthetic PQ BT.2020 16-bit source exported as ProRes probes bt2020 / smpte2084 / bt2020nc, 10-bit, with
  mastering-display and content-light side data. HEVC 10-bit PQ skipped (no encoder).
- Audio-only: 24-bit and 16-bit WAV equal the mixer output for the same blocks with a maximum difference of 0 LSB;
  M4A (AAC, `mp4` muxer as this build has no `ipod`) lasts 3.0000 s for a 3 s timeline (one AAC frame is 21.3 ms).
- Range 2–4 s of a 6 s timeline as ProRes 422 HQ: 2.000 s long; its first frame differs from the renderer's frame at
  2 s by 1.23 per channel on average (8-bit). The WAV of the same range equals the mix from 2 s within 1 LSB.
- `ove export Title master.mov --preset prores-422-hq-master` writes 1920×1080 ProRes and `pcm_s24le` from a 16:9
  project; size "match" on a 9:16 project gives 1080×1920.

### Group 3 (queue)

- The worker is `ove export --worker`, started detached (no console, own process group, no inherited standard
  handles) by whichever of `ove`, `ove mcp` or the app finds queued jobs and no live worker (`exports/worker.lock`
  holds its process id). Without clearing the inherit flag on Windows, a caller that captured `ove --detach` output
  waited for the worker to finish: found by the tests and fixed.
- `ove export … --detach --json` returned its job id in 53 ms (debug build); `ove export status <id> --wait` returned
  when the job was done.
- Three jobs queued while the worker lock was held ran strictly one after another in queue order once it was
  released; a fourth whose encoder cannot open failed with the encoder named, leaving no file and no `.partial`.
- Revision pinning: a job queued on a 2 s timeline still exported 2.0 s after the long item was deleted (the project
  then lasted 1 s); a job pinned to a revision that cannot be reconstructed failed with "… is no longer available;
  export again" and wrote nothing. A pin file in the bundle (`cache/pins/export-<id>.rev`) keeps `prune()` from
  removing the pinned revision (2,300-revision test) and is removed when the job finishes.
- Cancel of a running 120 s 1080p ProRes export: status `cancelled` 226 ms after `ove export cancel` started, output
  and partial file gone. Cancelling a finished job exits 1 (`job.finished`) and keeps the file.
- Killing the worker mid-export (`taskkill /F`): the next `ove export status --wait` put the job back (grace period
  set to 0 in the test; 30 s by default), started a worker, and the job finished on attempt 2 with a "restarted"
  warning. A job whose worker dies 3 times is marked failed (unit test).
- MCP (`ove mcp`, client "claude-code"): `export_video` returned a job id at once with `requestedBy: "claude-code"`,
  visible to `ove export queue`; `job_status` with `wait` returned the finished 1 s range; `cancel_job` cancelled a
  running ProRes job; unknown ids gave `job.unknown` naming the id; `wait: true` returned the export result; an
  impossible combination gave `export.invalid` and created no job.

### Group 4 (save range as media)

- Default intermediate on Windows: DNxHR HQ when the build has `dnxhd`, else ProRes 422 HQ (`prores_ks`); this
  machine's build has no `dnxhd`, so ProRes 422 HQ was written. macOS: ProRes 422 HQ. `small`: H.264 and AAC in MP4.
- Saving 5–12 s of a 16:9 project added one bundle-owned media item (`media/rendered/…mov`, 1920×1080, probes 7.0 s)
  and left every item unchanged.
- Render in place of 5–12 s over two video tracks (a title on each, one partly covered): the covered parts were lifted,
  one clip of the new media placed on the top chosen track, all in one revision; the composite at 8 s before and after
  has luma SSIM 0.9986; `ove undo` restored all items in one step and the rendered file stayed in the library.
- Only the chosen tracks are rendered for render in place; when another visual track has content in the range the
  render is ProRes 4444 with alpha. An offline media file inside the range refused the call before rendering
  (`export.refused`, naming the file): no revision, no media, no file.
