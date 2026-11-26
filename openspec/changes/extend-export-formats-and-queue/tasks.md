# Tasks: extend-export-formats-and-queue

## 1. FFmpeg build

- [ ] 1.1 Enable the HEVC, ProRes, DNxHD/DNxHR, libvpx-vp9, libopus and hardware AV1 encoders and the MOV, Matroska,
      WebM, iPod, WAV and FLAC muxers in `tools/ffmpeg/build.sh` for macOS and Windows; verify `check-license.sh` passes
      (no GPL or nonfree component) and record the installer size per platform in `sizes.txt` (≤ 60 MB).
- [ ] 1.2 Add a run-time encoder inventory to `ove doctor`; verify it lists the expected encoders on the macOS and
      Windows reference machines.

## 2. Settings model and encoders

- [ ] 2.1 Implement the settings model, validation and the preset table; verify unit tests for each valid preset and
      for each refused combination (codec/container, alpha, bit depth, range).
- [ ] 2.2 Implement per-codec encoder candidate lists with run-time filtering and reporting of the chosen encoder;
      verify a test with a mocked inventory picks the expected encoder and refuses HEVC when none exists.
- [ ] 2.3 Implement ProRes (all profiles), DNxHR (all profiles), HEVC 8/10-bit, VP9 and hardware AV1 encoding paths;
      verify an ffprobe matrix test (codec, profile, pixel format, colour tags) and SSIM ≥ 0.99 against the H.264
      golden export on a 10 s test timeline.
- [ ] 2.4 Implement alpha export for ProRes 4444 and VP9 with a transparent canvas; verify the alpha scenarios with a
      text-only timeline.
- [ ] 2.5 Implement the HDR tag path (10-bit conversion, BT.2020, PQ/HLG, mastering metadata) behind a check that the
      timeline produces HDR frames; verify the refusal scenario now and the tag scenario with a synthetic HDR frame
      source.
- [ ] 2.6 Implement audio-only export (M4A AAC with priming edit list, WAV 16/24-bit, FLAC) and range export; verify
      the WAV-matches-mix, AAC duration and range scenarios.
- [ ] 2.7 Update `docs/cli/export.md` with the settings model, presets and examples; verify the examples are executed by
      the CLI integration tests.

## 3. Queue

- [ ] 3.1 Implement job files, the detached worker, queue order, progress throttling and `.partial` rename; verify the
      several-jobs and failure scenarios in an integration test.
- [ ] 3.2 Implement revision pinning with a retention pin; verify the edit-while-waiting and revision-unavailable
      scenarios.
- [ ] 3.3 Implement cancel and restart recovery (dead worker detection, 3 attempts); verify by killing the worker
      process in a test and by the cancel scenarios (stop within 2 s measured).
- [ ] 3.4 Add `ove export --detach`, `ove export queue|status|cancel`; verify the detached scenario and the
      unknown-id error with exit codes.
- [ ] 3.5 Update MCP `export_video` (format, quality, range, wait, preset), `job_status` and `cancel_job` for export
      jobs; verify an MCP contract test for each, including the unknown-id error.

## 4. Save range as media

- [ ] 4.1 Implement save range as media (render, `AddMedia` as bundle-owned) and render in place (replace covered items
      in the same revision, delete the file on commit failure); verify the three scenarios, including SSIM ≥ 0.98 at a
      sample time and a single undo step.
- [ ] 4.2 Add `ove export --as-media [--replace]` and MCP `save_range_as_media`; verify CLI and MCP contract tests.

## 5. UI

- [ ] 5.1 Build the export dialog (presets, Advanced section, validation messages) and the Exports panel (progress,
      show file, cancel, export again); verify UI tests for preset choice, a refused combination and cancel.
- [ ] 5.2 Add "Save range as media" and "Render in place" to the timeline range menu with a confirmation for replace;
      verify a UI test that one undo restores the items.

## 6. Integration checks

- [ ] 6.1 In the built app on macOS and Windows: export ProRes 422 HQ, DNxHR HQ, HEVC 10-bit (where an encoder exists),
      VP9 WebM, ProRes 4444 with alpha and 24-bit WAV from the sample project; verify each file plays in the OS player
      and probes as expected.
- [ ] 6.2 In the built app on both OSes: queue three exports, quit, reopen; verify all finish, and record encode speed
      (× realtime) per codec at 1080p in design.md Results.
- [ ] 6.3 Agent path: an MCP client queues an export, polls `job_status`, and cancels a second one; verify on both OSes.
