# Proposal: add-background-tasks-and-media-derivatives

## Why

Background work is growing: thumbnails, captions, export, and soon proxies, skim sheets, analysis passes, indexing and
model inference. Today each feature runs its own job with its own cancel flag (`captions.rs`, `exporting.rs`), and
import probes and thumbnails files one after another on the command thread. Without one scheduler these jobs compete
for decoders, CPU and memory, playback drops frames, and nobody can see or cancel what is running. Media also has no
stable identity: caches are keyed by path, size and mtime, so moving or relinking a file throws away derived data.

## What Changes

- A background task scheduler shared by the app and the CLI: priorities (export first, interactive next, derivatives,
  then analysis), resource classes (decode sessions, inference, CPU-heavy) with their own limits, a memory budget,
  OS background thread priority, automatic pause during playback, export, battery power and thermal pressure,
  cooperative cancellation, and progress events.
- A Background Tasks popover in the status bar, `ove tasks list|cancel|wait`, and MCP `list_tasks` / `wait_tasks`.
- Captions, export and import thumbnails move onto the scheduler.
- A content identity for each media file, computed at import and kept across relinks; derived caches (transcripts,
  thumbnails, waveforms, future indexes) are keyed by it.
- Hover-skim: a sprite sheet per media file, served to the UI without decoding during the hover.
- Proxies: optional lower-resolution copies for heavy sources, used for playback, skim and analysis; export and
  agent inspection always use the originals. A per-project "Use proxies" setting.
- Settings → Storage rows for each cache (thumbnails, skim, proxies, transcripts, analysis) with size and Clear.

Out of scope: semantic search and its index (`add-semantic-media-search`), model installation
(`add-model-runtime`), timeline skim (rendered timeline previews), unused-media clean-up, a persisted task queue across
restarts (tasks are re-derived on open), the export queue UI and detached export worker
(`extend-export-formats-and-queue`, which builds on this scheduler).

References: docs/03-architecture-plan.md (roadmap item 5 media library: proxies; quality targets),
docs/spec/document-model.md (bundle layout: `cache/`, `proxies/`), docs/spec/cli.md (command conventions),
docs/spec/mcp-tools.md (tool conventions).

## Capabilities

### New Capabilities
- `background-tasks`: scheduling, budgets, pause rules, cancellation and visibility of background work.
- `media-derivatives`: content identity, skim sheets, proxies and the storage controls for derived data.

### Modified Capabilities
- None as requirements. Captions and export keep their behaviour; they now run through the scheduler.

## Impact

- New crate `ove-tasks`; changes in `apps/desktop/src-tauri` (`projects.rs` import, `captions.rs`, `exporting.rs`),
  `ove-transcribe` cache keys, `ove-media` (decoder option to skip non-reference frames for derivative work), the media
  panel UI and a status-bar popover.
- Media registry entries gain a `contentId` field (an additive document field, written at import as part of the
  existing add-media op).
- Depends on nothing. Used by: `add-model-runtime` (inference jobs), `add-semantic-media-search`,
  `add-transcript-editing`, `add-audio-mixing-and-monitoring`, `add-audio-intelligence`,
  `extend-export-formats-and-queue`.
