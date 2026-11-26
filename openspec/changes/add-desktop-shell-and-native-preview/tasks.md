# Tasks: add-desktop-shell-and-native-preview

## 1. Toolchain, workspace and CI

- [x] 1.1 Install Rust stable via rustup (record exact version in `rust-toolchain.toml`), Node LTS for the UI build, and
      pnpm; verify `cargo --version` and `node --version` match the pinned versions.
- [x] 1.2 Create the Cargo workspace (`crates/ove-media`, `ove-playback`, `ove-gpu`, `ove-bench`, `ove-core` stub) with
      edition 2024, shared lints (`clippy -D warnings`), and `deny.toml` with the allowed licence list; verify
      `cargo build --workspace` and `cargo deny check` pass.
      Status: Done: workspace builds; `cargo deny check` passes (harden-for-alpha 6.4).
- [x] 1.3 Add GitHub Actions CI on `macos-15` (arm64) and `windows-2025` (x64): fmt, clippy, tests, cargo-deny, UI
      typecheck/lint/tests, debug app build; verify a PR shows all jobs green on both OSes.
      Status: Workflows written (`.github/workflows/ci.yml`, macOS, Windows, Ubuntu). Green run: P.1.
- [x] 1.4 Add `tools/ffmpeg` reproducible LGPL build (pinned FFmpeg 9.x source + checksum, minimal configure for the
      formats in specs/media-playback, VideoToolbox / D3D11VA enabled) producing shared libraries for both targets; CI
      fails if configure output contains `--enable-gpl` or a nonfree component; verify `ffmpeg -buildconf` output is
      stored as a CI artefact and the libraries' total size is recorded.
      Status: Linux and Windows MinGW cross builds done; licence gate verified (design.md Results). Native builds: P.2.

## 2. Media decode core (headless)

- [x] 2.1 Build the real-media test matrix: list of files (ProRes 422 Proxy/LT/HQ/4444, DNxHR HQ in MXF, H.264 4K60,
      HEVC 10-bit 4:2:0 and 4:2:2, iPhone HEVC HDR VFR, AV1, VP9, audio-only AAC/PCM, a truncated file) with sha256 in
      `tests/media/MANIFEST.json`; files stored outside git (download script); verify the script fetches and checksums
      all files.
      Status: Synthetic matrix (20 files, `tests/media/MANIFEST.json`, `media.py`). Real footage: P.3.
- [x] 2.2 Implement probe (container, streams, duration, frame rate incl. VFR detection, codec/profile, audio layout) in
      `ove-media`; verify with snapshot tests against `ffprobe` output for every matrix file.
- [x] 2.3 Implement software decode with a bounded frame queue and exact seek (keyframe + decode-forward); verify frame
      checksums for 200 random seek targets per matrix file match an `ffmpeg -vf framemd5` reference.
- [x] 2.4 Implement hardware decode (VideoToolbox, D3D11VA) with automatic fallback, including mid-stream fallback on
      decoder error (fault-injected in tests); verify the same checksum suite passes on the hardware path where
      supported, and the fallback test resumes at the same frame.
      Status: Code done (`HwAccel`, `create_hw_device`, `FallbackDecoder`); fault-injected mid-stream fallback verified
      on Linux. Hardware path: P.4.
- [x] 2.5 Implement audio decode + resample to the device format; verify sample-accurate output against an ffmpeg
      reference (PCM diff ≤ 1 LSB after resampling) for each audio codec in the matrix.

## 3. Playback engine and benchmark (headless)

- [x] 3.1 Implement the clock (audio-master / system fallback), scheduler, frame dropping and transport state machine
      (play, pause, step, shuttle −2×…2×, loop, seek) in `ove-playback`; verify with unit tests on a simulated clock
      covering every transport transition and late-frame dropping.
- [x] 3.2 Implement audio output with device-change handling (cpal); verify on both OSes by switching output devices
      during playback (manual test recorded in Results) and with a test for the no-device fallback.
      Status: Code done (`ove-playback` output.rs); no-device fallback verified on Linux. Device switching: P.5.
- [x] 3.3 Implement `ove-bench` (headless playback + 100 random seeks, JSON metrics: decode path, presented/dropped
      frames, A/V offset stats, seek p50/p95, peak memory); verify it runs on every matrix file on both CI OSes and
      writes valid JSON against a checked-in schema.
      Status: Done on Linux for every matrix file; reports validate against `crates/ove-bench/schema.json`. Both OSes:
      P.6.

## 4. Desktop shell and bridge

- [x] 4.1 Scaffold the Tauri 2 app with React 19 + TypeScript + Vite UI; dark window, macOS hidden-inset title bar,
      window state persistence with off-screen clamping, 1024×640 minimum; verify the first-launch and
      disconnected-display scenarios manually on both OSes (record in Results).
      Status: Code done (Tauri 2 + React 19, `window_state.rs`). Manual scenarios: P.7.
- [x] 4.2 Add the typed bridge (types generated from Rust, committed bindings, CI diff check) with `AppError{code,
      message}`; verify a test command's error reaches the UI as a structured error and the contract-drift check fails
      a deliberately stale binding.
- [x] 4.3 Add single instance, "Open with", drag-and-drop onto window and Dock/taskbar routing; verify each scenario in
      specs/desktop-shell on both OSes.
      Status: Code done (single-instance plugin, open-with, drag-and-drop routing in `lib.rs`). Scenarios: P.8.
- [x] 4.4 Add rotating logs (startup line with OS, GPU adapter, decode capabilities) and panic containment at worker
      boundaries; verify a forced worker panic shows an error and the window stays usable, and the log contains the
      backtrace.
      Status: Code done (`logging.rs` startup line with OS, GPU, decode; panic containment verified by harden-for-alpha
      1.1). Forced panic in the built app: P.9.
- [ ] 4.5 Measure cold start to interactive UI; verify ≤ 1.5 s on both reference machines (record in Results).
      Windows: 409–508 ms, release, warm cache (design.md Results). Left: macOS, first start after reboot.

## 5. Native viewer (R1 gate)

- [x] 5.1 Implement `ove-gpu`: device creation, surface management, NV12/P010/RGBA frame upload (zero-copy import where
      available), letterboxed viewer render, device-loss recovery; verify with a golden-image test of a rendered
      frame and a forced device-removal test on Windows.
      Status: Code done (`ove-gpu`, golden-image tests in `crates/ove-gpu/tests/golden.rs`, device-lost callback).
      Device removal on Windows: P.10.
- [x] 5.2 Implement viewer layering candidate 1 (under + transparent web view) on macOS and Windows with rect/scale sync
      from the UI; verify all placement scenarios (resize, display/scale change, fullscreen, menu over viewer) with
      screenshot-based UI tests on both OSes.
      Status: Candidate 1 implemented on both OSes. Windows: WebView2's transparent background does not show the native
      viewer (commit 2a17c44), so Windows defaults to candidate 3 (frame stream); candidate 1 stays behind
      `OVE_VIEWER_LAYERING`. macOS placement tests: P.11.
- [x] 5.3 If candidate 1 fails any placement or performance scenario on an OS, implement candidate 2, then candidate
      3, for that OS behind the same viewer interface; verify the same tests. Record each candidate's results.
      Status: Windows uses candidate 3 (frame stream) by default; `tools/preview-check.sh` checks its picture on the
      real screen. Candidate 2 not built. Decision: P.12.
- [x] 5.4 Wire playback into the viewer with transport UI (buttons, J/K/L, arrow-key step, Home/End, scrub bar,
      timecode entry, loop) and the diagnostics overlay; verify every media-playback transport scenario in the built app
      on both OSes.
      Status: Done and verified under Xvfb on Linux (design.md Results, frame-step check). Both OSes: P.13.

## 6. Gate evaluation (integration)

- [ ] 6.1 Run `ove-bench` and in-app playback for the full matrix on the macOS and Windows reference machines, with the
      UI idle and with a panel animating; record all numbers in design.md Results and compare to the
      media-playback performance requirement.
- [ ] 6.2 Measure the release build installer size and idle RAM with one file open on both OSes; record in Results.
      Windows: NSIS 25.5 MB, MSI 35.2 MB; idle RAM 55 MB on Home, 329 MB (720p) to 492 MB (4K) with a file open, over
      the 250 MB note with a file (design.md Results). Left: macOS.
- [ ] 6.3 Write the gate decision in design.md (chosen layering per OS, unmet targets, follow-ups) and present it to the
      owner for sign-off before changes 5–7 start.

## Pending verification (macOS, Windows, owner)

Code for the tasks above is done; these checks remain. Each needs a platform, a CI run or the owner.

- [ ] P.1 One green CI run on macOS and Windows (blocked: GitHub Actions jobs do not start on this account; same as
      harden-for-alpha 6.5).
- [ ] P.2 Native FFmpeg builds on macOS arm64 (VideoToolbox) and Windows x64 MSVC (D3D11VA); `buildconf.txt` stored as a
      CI artefact; record library sizes.
      Windows: done 2026-11-26 (MSVC, D3D11VA, 11.0 MB; design.md Results). Left: macOS, CI artefact.
- [ ] P.3 Owner provides real camera footage (Apple ProRes, iPhone HDR, camera MXF, 1-hour long-GOP file); add it under
      `real` in the manifest.
- [ ] P.4 Checksum suite on the hardware path: VideoToolbox (macOS) and D3D11VA (Windows); mid-stream fallback resumes
      at the same frame on real hardware.
      Windows: done 2026-11-26 (`tests/hardware.rs`, 8 files on D3D11VA incl. AV1, identical to software). Left: macOS.
- [ ] P.5 Switch output devices during playback on macOS and Windows; record the manual test in Results.
- [ ] P.6 `ove-bench` on every matrix file on macOS and Windows.
      Windows: done 2026-11-26 (design.md Results). Left: macOS.
- [ ] P.7 First-launch and disconnected-display scenarios on macOS and Windows; record in Results.
- [ ] P.8 Single instance, Open with, drag-and-drop and Dock/taskbar routing: every specs/desktop-shell scenario on
      macOS and Windows.
      Windows: second launch and launch with a file (focus, one process, file routed) checked by hand; drop on the
      window covered by the app e2e suite (17/17 flows pass). Left: taskbar drop, macOS.
- [ ] P.9 Forced worker panic in the built app on macOS and Windows: error shown, window usable, backtrace in the log.
      Windows: done 2026-11-26 by the app e2e flow "resilience: a forced panic in a worker and in the viewer" (also
      runs on Linux CI). Left: macOS.
- [ ] P.10 Forced device-removal test on Windows.
- [ ] P.11 Placement scenarios (resize, display/scale change, fullscreen, menu over viewer) for candidate 1 on macOS.
- [ ] P.12 Decide whether Windows needs candidate 2 or keeps candidate 3, from the 6.1 performance numbers.
- [ ] P.13 Every media-playback transport scenario in the built app on macOS and Windows.
