# Design: add-background-tasks-and-media-derivatives

## Context

See proposal.md (Why). Today: one caption job and one export job at a time, each with an `AtomicBool` cancel flag;
import probes and thumbnails synchronously; caches keyed by path + size + mtime (`ove-transcribe/src/cache.rs`);
`proxies/` is reserved in `ove-store/src/bundle.rs` but unused. Requirements: specs/background-tasks,
specs/media-derivatives.

## Goals / Non-Goals

**Goals:**
- One scheduler for all background work, usable from the app and from the headless CLI.
- Playback never drops frames because of background work.
- Derived data survives relinks.

**Non-Goals:**
- A durable job queue (results are idempotent and re-derived instead).
- Distributed or remote workers.

## Decisions

### D1. Scheduler model
Crate `ove-tasks`. A task is `{id, project, kind, priority, class, est_mem, inputs_key, run(ctx)}`. Priorities: P0
export, P1 interactive, P2 derivatives (thumbnails, skim, proxies), P3 analysis and indexing. Classes with limits:
`decode` (2 hardware sessions), `inference` (1), `cpu` (cores − 2, minimum 1), `io` (4). The memory budget is
min(1.5 GB, 20 % of physical RAM). A waiting task's effective priority rises one level per 5 minutes waited.
Alternatives: a general async runtime pool with no classes (cannot stop two 4K decodes from fighting); one global
serial queue (too slow for many small thumbnails).

### D2. OS priority and pause rules
Worker threads set background priority (macOS QoS utility/background; Windows `THREAD_MODE_BACKGROUND_BEGIN`). The
scheduler subscribes to playback state, export state, power source, low-power mode and thermal state; P2/P3 tasks
pause at their next checkpoint when any is active. The person can override ("run anyway") per pause reason.
Alternative: lower priority only, no pause (background decode still steals decoder sessions and memory bandwidth from
playback).

### D3. Cancellation and partial outputs
Each task gets a cancel token checked per frame or per chunk. Outputs are written to `*.part` and renamed on success.
Tasks in another process (model worker) are cancelled by message, then killed after 1 s.

### D4. No saved queue
Every task's output path is a function of its inputs key. On project open, a scan queues tasks whose outputs are
missing. Quitting drops the queue. Alternative: a persisted queue (adds a recovery protocol and stale-entry bugs for no
gain, since outputs are idempotent).

### D5. Content identity
`contentId` = SHA-256 over the file size and the first, middle and last 1 MiB. It is computed at import and stored on
the media registry entry as part of the existing add-media op, so it creates no extra revision. Size + mtime stay as a
fast "has it changed" check that triggers a recompute. Caches move to `<bundle>/cache/<kind>/<contentId>…`; transcript
caches migrate by re-keying on first open.
Alternatives: full-file hash (minutes for large files); path + size + mtime (breaks on relink, the current problem).

### D6. Skim sheets
P2 task: decode 60–120 evenly spaced frames at 160 px wide (skipping non-reference frames where the decoder allows),
pack them into one WebP sprite sheet plus a JSON of times. The UI maps pointer x to a tile with CSS background-position.
Files are served through a custom URI protocol, not base64 over IPC.
Alternatives: decode on hover (latency and decoder churn); short preview videos (larger files, slower to seek).

### D7. Proxies
P2 task for sources ≥ 4K, 10-bit long-GOP, or failing a real-time decode probe. Output: 1080p H.264 8-bit, keyframe
every 12 frames, audio copied, written to `proxies/<contentId>_1080.mp4` with the hardware encoders already used by
export. HDR sources reuse the tone-mapping in `ove-media`. The project setting "Use proxies" is a project field set
through the normal update-project op; proxy paths are derived, never stored in the document. Playback, skim and
analysis read proxies when the setting is on; export and agent `look`/`grab_frame` always read originals so agent
inspection matches export.
Alternatives: ProRes Proxy (large files, no hardware encoder on Windows); proxies stored in the document (breaks the
derived-data rule and merges).

### D8. Surfaces
Status-bar popover (running, waiting, paused with reason, progress, cancel). CLI `ove tasks list|cancel|wait [--json]`
working on bundles headless. MCP `list_tasks`, `wait_tasks` (with time-out), `cancel_task`. Progress events throttled to
4 Hz. Settings → Storage rows per cache kind with size and Clear.

## Risks / Trade-offs

- Priority inversion: a P3 task holds a decode session a P0 export needs. Mitigation: per-class semaphores with
  preemption at checkpoints for P0. Detection: soak test with export started during heavy indexing; export start
  latency ≤ 1 s.
- A scheduler bug stalls export. Mitigation: export can bypass the queue with a watchdog after 5 s of no start.
  Detection: fault-injection test.
- Sampled content identity collides for edited files of identical size. Mitigation: size + mtime change triggers a
  recompute. Detection: unit test with a byte change outside the sampled regions plus mtime change.
- Proxy colour mismatch. Mitigation: shared tone-map path; golden SSIM check proxy vs original ≥ 0.97 at 1080p.
  Detection: that test.
- Disk use by proxies and skim sheets. Mitigation: Storage controls; skim sheets about 150 KB per file. Detection:
  Storage totals in the UI.

## Migration Plan

On first open after upgrade: compute `contentId` for each media entry in a P2 task and re-key existing caches. Projects
opened in an older build ignore the new field (unknown fields round-trip).

## Open Questions

- Waves proceed in master-roadmap order; this change is wave 1 and should land before the model-based features.
- Background transcription of all imported media stays off by default; the owner may want it on.
- Pause-on-battery default (on) may need a setting for desktop workstations on UPS power.

## Results

(Filled in during apply: export start latency under load, dropped frames during 4K playback with indexing running,
cancel latency, skim first-tile latency, proxy encode speed, idle RAM with an empty queue — both reference machines.)

Scheduler core, measured by `cargo test -p ove-tasks` on the Windows 11 x64 development machine (debug build, test
profile). These are unit-level numbers with test tasks that check in every 5–10 ms; the built-app numbers on the
reference machines are still open (tasks 7.1–7.3).

| Measure | Result | Target |
|---|---|---|
| Cancel of a running task to its finished state | 2.8 ms | ≤ 200 ms |
| Playback pause to the task reported paused | 1.5 ms | ≤ 200 ms |
| Export start while an analysis task holds the only decode slot | 4.8 ms | ≤ 1 s |
| Export start through the watchdog with a stalled queue (300 ms watchdog in the test, 5 s in the app) | ≈ 300 ms | watchdog + margin |
| Progress events from a task reporting every millisecond for 1 s | 4–5 | ≤ 4 per second (+ the first) |
| Analysis task under continuous interactive load (simulated clock) | starts at 600 s | ≤ 10 min |

Existing jobs on the scheduler, measured by `cargo test -p ove-desktop` on the same machine (160×90 test clips made by
our own exporter):

| Measure | Result |
|---|---|
| 200-file import: all probes (P1 tasks, 4 at a time) | 145 ms |
| 200-file import: every file in the document (one revision) | 147 ms after start, before any thumbnail |
| 200-file import: all 200 thumbnails (P2 tasks, 2 decode sessions) | 2.3 s after the list |
| Export with a stalled queue, 5 s watchdog | done after 5.14 s, marked as bypassed |

Import waits for its probes: a clip's length decides where it goes on the timeline, and probes are quick. Probes carry
a 2 s watchdog so a paused or busy queue never holds an import up. Thumbnails are not awaited.

Content identity as built (group 3):

- `contentId` is the first 32 hex characters (128 bits) of SHA-256 over a version tag, the size, and the first,
  middle and last MiB; files of 3 MiB or less are hashed whole. It is written with the add-media op at import (app and
  `ove media add`) and with the update-media op at relink.
- The collision risk (an edit between the sampled regions that keeps the size) is handled where it is detected: when
  the size or time of a file at the same path changed but its sampled id did not, the stored id becomes a hash of the
  whole file, so derived data is rebuilt. Copies and moves keep the sampled id.
- Older projects: on open, a P2 "Media identities" task computes the missing ids and stores them as one revision
  attributed to migration (`Source::Migration`). The next media check moves thumbnails named by media id to
  `cache/thumbs/<contentId>.png` and renames transcripts keyed by path, size and time to
  `cache/transcripts/<contentId>-<model>-<language>.json`. Captions also take an unmigrated transcript under the old
  key, so nothing is transcribed again even before the migration task runs.
- Thumbnails other features write by media id (sample project, generation, interchange, `ove media` thumbnails) are
  found under the media id and moved on the next check, so those features did not need changing.
