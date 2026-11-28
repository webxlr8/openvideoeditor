# Tasks: extend-export-formats-and-queue

## 1. FFmpeg build

- [ ] 1.1 Enable the HEVC, ProRes, DNxHD/DNxHR, libvpx-vp9, libopus and hardware AV1 encoders and the MOV, Matroska,
      WebM, iPod, WAV and FLAC muxers in `tools/ffmpeg/build.sh` for macOS and Windows; verify `check-license.sh` passes
      (no GPL or nonfree component) and record the installer size per platform in `sizes.txt` (≤ 60 MB).
      Open: flags added and the licence check passes on the current build; the rebuild (MSYS2, vcpkg libvpx, opus,
      ffnvcodec, amd-amf, libvpl) and installer sizes need the build machines.
- [ ] 1.2 Add a run-time encoder inventory to `ove doctor`; verify it lists the expected encoders on the macOS and
      Windows reference machines.
      Open: implemented and checked on Windows (see design.md Results); macOS reference machine not available.

## 2. Settings model and encoders

- [x] 2.1 Implement the settings model, validation and the preset table; verify unit tests for each valid preset and
      for each refused combination (codec/container, alpha, bit depth, range).
- [x] 2.2 Implement per-codec encoder candidate lists with run-time filtering and reporting of the chosen encoder;
      verify a test with a mocked inventory picks the expected encoder and refuses HEVC when none exists.
- [ ] 2.3 Implement ProRes (all profiles), DNxHR (all profiles), HEVC 8/10-bit, VP9 and hardware AV1 encoding paths;
      verify an ffprobe matrix test (codec, profile, pixel format, colour tags) and luma SSIM ≥ 0.99 (ProRes Proxy ≥ 0.98)
      against the H.264 golden export on a 10 s test timeline.
      Open: ProRes (all profiles) verified; HEVC, DNxHR, VP9 and AV1 paths are implemented but this build or machine has
      no encoder for them, so their matrix cases are skipped.
- [ ] 2.4 Implement alpha export for ProRes 4444 and VP9 with a transparent canvas; verify the alpha scenarios with a
      text-only timeline.
      Open: ProRes 4444 alpha verified; VP9 alpha needs libvpx and the WebM muxer (not in this build).
- [x] 2.5 Implement the HDR tag path (10-bit conversion, BT.2020, PQ/HLG, mastering metadata) behind a check that the
      timeline produces HDR frames; verify the refusal scenario now and the tag scenario with a synthetic HDR frame
      source.
      Note: tags verified on ProRes; HEVC 10-bit PQ has no encoder on this machine.
- [x] 2.6 Implement audio-only export (M4A AAC with priming edit list, WAV 16/24-bit, FLAC) and range export; verify
      the WAV-matches-mix, AAC duration and range scenarios.
      Note: the FLAC path has no encoder or muxer in this build; WAV, M4A and range verified.
- [x] 2.7 Update `docs/cli/export.md` with the settings model, presets and examples; verify the examples are executed by
      the CLI integration tests.

## 3. Queue

- [x] 3.1 Implement job files, the detached worker, queue order, progress throttling and `.partial` rename; verify the
      several-jobs and failure scenarios in an integration test.
- [x] 3.2 Implement revision pinning with a retention pin; verify the edit-while-waiting and revision-unavailable
      scenarios.
- [x] 3.3 Implement cancel and restart recovery (dead worker detection, 3 attempts); verify by killing the worker
      process in a test and by the cancel scenarios (stop within 2 s measured).
- [x] 3.4 Add `ove export --detach`, `ove export queue|status|cancel`; verify the detached scenario and the
      unknown-id error with exit codes.
- [x] 3.5 Update MCP `export_video` (format, quality, range, wait, preset), `job_status` and `cancel_job` for export
      jobs; verify an MCP contract test for each, including the unknown-id error.

## 4. Save range as media

- [x] 4.1 Implement save range as media (render, `AddMedia` as bundle-owned) and render in place (replace covered items
      in the same revision, delete the file on commit failure); verify the three scenarios, including SSIM ≥ 0.98 at a
      sample time and a single undo step.
- [x] 4.2 Add `ove export --as-media [--replace]` and MCP `save_range_as_media`; verify CLI and MCP contract tests.

## 5. UI

- [x] 5.1 Build the export dialog (presets, Advanced section, validation messages) and the Exports panel (progress,
      show file, cancel, export again); verify UI tests for preset choice, a refused combination and cancel.
- [x] 5.2 Add "Save range as media" and "Render in place" to the timeline range menu with a confirmation for replace;
      verify a UI test that one undo restores the items.
      Note: the menu opens with a right-click on the marked range in the ruler, and also offers "Export range…".

## 6. Integration checks

- [ ] 6.1 In the built app on macOS and Windows: export ProRes 422 HQ, DNxHR HQ, HEVC 10-bit (where an encoder exists),
      VP9 WebM, ProRes 4444 with alpha and 24-bit WAV from the sample project; verify each file plays in the OS player
      and probes as expected.
      Open: Windows half done for ProRes 422 HQ, ProRes 4444 with alpha, H.264 and 24-bit WAV (probe and full decode);
      DNxHR, VP9 and HEVC have no encoder in this build or machine; OS-player playback and the macOS half need a person
      and the macOS machine.
- [ ] 6.2 In the built app on both OSes: queue three exports, quit, reopen; verify all finish, and record encode speed
      (× realtime) per codec at 1080p in design.md Results.
      Open: Windows half done (e2e flow "export queue"; debug-build speeds in Results); macOS half not available here.
- [ ] 6.3 Agent path: an MCP client queues an export, polls `job_status`, and cancels a second one; verify on both OSes.
      Open: Windows half done (e2e flow "agent export"); macOS half not available here.
