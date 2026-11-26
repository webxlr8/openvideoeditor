# Tasks: add-background-tasks-and-media-derivatives

## 1. Scheduler core

- [ ] 1.1 Create crate `ove-tasks` with task, priority, class, memory budget and age bump; verify unit tests for
      ordering, class limits, budget admission, the "runs alone" case and no starvation (simulated clock).
- [ ] 1.2 Add cancel tokens, `.part` outputs and rename-on-success helpers; verify cancel stops a test task within
      200 ms and leaves no visible output.
- [ ] 1.3 Add OS background priority on macOS and Windows; verify a test reads back the thread priority on both OSes.
- [ ] 1.4 Add pause sources (playback, export, power, low-power, thermal) and the person's override; verify tests with
      injected state changes pause and resume tasks at checkpoints.
- [ ] 1.5 Document the scheduler for feature authors in `docs/cli/tasks.md` (task kinds, priorities, how to add one);
      verify the docs' CLI examples run in tests.

## 2. Existing jobs onto the scheduler

- [ ] 2.1 Move import probe and thumbnails to P1/P2 tasks; import returns before they finish; verify a 200-file import
      test lists all files at once and fills thumbnails as tasks complete.
- [ ] 2.2 Move caption jobs to P3 tasks with unchanged results and errors; verify the existing caption tests pass.
- [ ] 2.3 Move export to a P0 task with the watchdog bypass; verify the existing export tests pass and a fault-injected
      stalled queue still starts the export within 5 s.

## 3. Content identity

- [ ] 3.1 Compute `contentId` at import and write it in the add-media op; verify tests: rename and move keep it, a
      content change alters it, the field round-trips in older readers.
- [ ] 3.2 Re-key transcript and thumbnail caches to `contentId` with migration on first open; verify an upgrade test
      keeps existing transcripts without re-transcribing.

## 4. Skim sheets

- [ ] 4.1 Implement the skim-sheet task (frame choice, 160 px tiles, WebP sheet, times JSON) with a decoder option to
      skip non-reference frames; verify a golden test of tile times and that decode work drops on long-GOP fixtures.
- [ ] 4.2 Serve sheets through a custom URI protocol and add hover-skim to media thumbnails; verify a UI test: no IPC
      calls while skimming and first tile ≤ 16 ms after pointer move.

## 5. Proxies

- [ ] 5.1 Implement proxy eligibility (≥ 4K, 10-bit long-GOP, real-time probe) and the proxy encode task with hardware
      encoders; verify encodes on fixtures and SSIM ≥ 0.97 against the original at 1080p, including an HDR source.
- [ ] 5.2 Add the "Use proxies" project setting and switch playback, skim and analysis to proxies; keep export and
      agent frames on originals; verify a test that export output is identical with proxies on and off, and that a
      missing proxy falls back to the original.

## 6. Surfaces

- [ ] 6.1 Add the status-bar Background Tasks popover (states, progress, pause reason, cancel, run anyway); verify UI
      tests for each state.
- [ ] 6.2 Add `ove tasks list|cancel|wait` (headless on bundles) and MCP `list_tasks`, `wait_tasks`, `cancel_task`;
      verify CLI and MCP contract tests including time-out results.
- [ ] 6.3 Add Settings → Storage rows per cache kind with size and Clear (cancel-then-clear); verify UI tests that
      clearing never touches media or the document.

## 7. Integration checks

- [ ] 7.1 In the built app on macOS and Windows: play a 4K 10-bit clip for 60 s while indexing-like P3 tasks run;
      verify zero dropped frames caused by background work and record numbers in design.md Results.
- [ ] 7.2 Start an export during a heavy background load on both OSes; verify the export starts within 1 s.
- [ ] 7.3 Import 200 files, skim them, build proxies, relink the folder, and reopen the project on both OSes; verify
      derived data is reused and idle RAM with an empty queue stays ≤ 250 MB.
