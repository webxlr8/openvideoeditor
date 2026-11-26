# Tasks: add-background-tasks-and-media-derivatives

## 1. Scheduler core

- [x] 1.1 Create crate `ove-tasks` with task, priority, class, memory budget and age bump; verify unit tests for
      ordering, class limits, budget admission, the "runs alone" case and no starvation (simulated clock).
- [x] 1.2 Add cancel tokens, `.part` outputs and rename-on-success helpers; verify cancel stops a test task within
      200 ms and leaves no visible output.
- [ ] 1.3 Add OS background priority on macOS and Windows; verify a test reads back the thread priority on both OSes.
- [x] 1.4 Add pause sources (playback, export, power, low-power, thermal) and the person's override; verify tests with
      injected state changes pause and resume tasks at checkpoints.
- [ ] 1.5 Document the scheduler for feature authors in `docs/cli/tasks.md` (task kinds, priorities, how to add one);
      verify the docs' CLI examples run in tests.
      (Written; its Rust example runs as a doc test. The `ove tasks` examples run once 6.2 adds the command.)

## 2. Existing jobs onto the scheduler

- [x] 2.1 Move import probe and thumbnails to P1/P2 tasks; import returns before they finish; verify a 200-file import
      test lists all files at once and fills thumbnails as tasks complete.
      (Probes run as P1 tasks, four at a time, and import waits for them because a clip's length decides where it
      goes; import returns before thumbnails, which are P2 tasks. Missing thumbnails are queued again on open.
      `projects::tests::large_import_lists_every_file_at_once_and_thumbnails_follow`.)
- [x] 2.2 Move caption jobs to P3 tasks with unchanged results and errors; verify the existing caption tests pass.
- [x] 2.3 Move export to a P0 task with the watchdog bypass; verify the existing export tests pass and a fault-injected
      stalled queue still starts the export within 5 s.

## 3. Content identity

- [x] 3.1 Compute `contentId` at import and write it in the add-media op; verify tests: rename and move keep it, a
      content change alters it, the field round-trips in older readers.
      (`ove-store` media tests; the app's import and `ove media add` / `relink` write it. `contentId` is one additive
      optional field in the registry schema.)
- [x] 3.2 Re-key transcript and thumbnail caches to `contentId` with migration on first open; verify an upgrade test
      keeps existing transcripts without re-transcribing.
      (`projects::tests::upgrade_keeps_derived_data_and_relink_reuses_it`,
      `ove-transcribe` `cache::tests::legacy_entries_migrate_to_the_content_id`.)

## 4. Skim sheets

- [x] 4.1 Implement the skim-sheet task (frame choice, 160 px tiles, WebP sheet, times JSON) with a decoder option to
      skip non-reference frames; verify a golden test of tile times and that decode work drops on long-GOP fixtures.
      (The sheet is a JPEG, not WebP: see design.md Results. `tasks::tests::skim_sheet_tiles_and_decode_work` uses a
      60 s clip with a keyframe every 2 s made by our exporter, as the long-GOP fixture; the matrix fixtures need
      python3 and a fixture ffmpeg this machine lacks.)
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
