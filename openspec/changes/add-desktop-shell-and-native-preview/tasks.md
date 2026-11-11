# Tasks: add-desktop-shell-and-native-preview

## 1. Toolchain, workspace and CI

- [x] 1.1 Install Rust stable via rustup (record exact version in `rust-toolchain.toml`), Node LTS for the UI build, and
      pnpm; verify `cargo --version` and `node --version` match the pinned versions.
- [ ] 1.2 Create the Cargo workspace (`crates/ove-media`, `ove-playback`, `ove-gpu`, `ove-bench`, `ove-core` stub) with
      edition 2024, shared lints (`clippy -D warnings`), and `deny.toml` with the allowed licence list; verify
      `cargo build --workspace` and `cargo deny check` pass.
- [ ] 1.3 Add GitHub Actions CI on `macos-15` (arm64) and `windows-2025` (x64): fmt, clippy, tests, cargo-deny, UI
      typecheck/lint/tests, debug app build; verify a PR shows all jobs green on both OSes.
- [ ] 1.4 Add `tools/ffmpeg` reproducible LGPL build (pinned FFmpeg 9.x source + checksum, minimal configure for the
      formats in specs/media-playback, VideoToolbox / D3D11VA enabled) producing shared libraries for both targets; CI
      fails if configure output contains `--enable-gpl` or a nonfree component; verify `ffmpeg -buildconf` output is
      stored as a CI artefact and the libraries' total size is recorded.

## 2. Media decode core (headless)

- [ ] 2.1 Build the real-media test matrix: list of files (ProRes 422 Proxy/LT/HQ/4444, DNxHR HQ in MXF, H.264 4K60,
      HEVC 10-bit 4:2:0 and 4:2:2, iPhone HEVC HDR VFR, AV1, VP9, audio-only AAC/PCM, a truncated file) with sha256 in
      `tests/media/MANIFEST.json`; files stored outside git (download script); verify the script fetches and checksums
      all files.
- [x] 2.2 Implement probe (container, streams, duration, frame rate incl. VFR detection, codec/profile, audio layout) in
      `ove-media`; verify with snapshot tests against `ffprobe` output for every matrix file.
- [x] 2.3 Implement software decode with a bounded frame queue and exact seek (keyframe + decode-forward); verify frame
      checksums for 200 random seek targets per matrix file match an `ffmpeg -vf framemd5` reference.
- [ ] 2.4 Implement hardware decode (VideoToolbox, D3D11VA) with automatic fallback, including mid-stream fallback on
      decoder error (fault-injected in tests); verify the same checksum suite passes on the hardware path where
      supported, and the fallback test resumes at the same frame.
- [x] 2.5 Implement audio decode + resample to the device format; verify sample-accurate output against an ffmpeg
      reference (PCM diff ≤ 1 LSB after resampling) for each audio codec in the matrix.

## 3. Playback engine and benchmark (headless)

- [x] 3.1 Implement the clock (audio-master / system fallback), scheduler, frame dropping and transport state machine
      (play, pause, step, shuttle −2×…2×, loop, seek) in `ove-playback`; verify with unit tests on a simulated clock
      covering every transport transition and late-frame dropping.
- [ ] 3.2 Implement audio output with device-change handling (cpal); verify on both OSes by switching output devices
      during playback (manual test recorded in Results) and with a test for the no-device fallback.
- [ ] 3.3 Implement `ove-bench` (headless playback + 100 random seeks, JSON metrics: decode path, presented/dropped
      frames, A/V offset stats, seek p50/p95, peak memory); verify it runs on every matrix file on both CI OSes and
      writes valid JSON against a checked-in schema.

## 4. Desktop shell and bridge

- [ ] 4.1 Scaffold the Tauri 2 app with React 19 + TypeScript + Vite UI; dark window, macOS hidden-inset title bar,
      window state persistence with off-screen clamping, 1024×640 minimum; verify the first-launch and
      disconnected-display scenarios manually on both OSes (record in Results).
- [ ] 4.2 Add the typed bridge (types generated from Rust, committed bindings, CI diff check) with `AppError{code,
      message}`; verify a test command's error reaches the UI as a structured error and the contract-drift check fails
      a deliberately stale binding.
- [ ] 4.3 Add single instance, "Open with", drag-and-drop onto window and Dock/taskbar routing; verify each scenario in
      specs/desktop-shell on both OSes.
- [ ] 4.4 Add rotating logs (startup line with OS, GPU adapter, decode capabilities) and panic containment at worker
      boundaries; verify a forced worker panic shows an error and the window stays usable, and the log contains the
      backtrace.
- [ ] 4.5 Measure cold start to interactive UI; verify ≤ 1.5 s on both reference machines (record in Results).

## 5. Native viewer (R1 gate)

- [ ] 5.1 Implement `ove-gpu`: device creation, surface management, NV12/P010/RGBA frame upload (zero-copy import where
      available), letterboxed viewer render, device-loss recovery; verify with a golden-image test of a rendered
      frame and a forced device-removal test on Windows.
- [ ] 5.2 Implement viewer layering candidate 1 (under + transparent web view) on macOS and Windows with rect/scale sync
      from the UI; verify all placement scenarios (resize, display/scale change, fullscreen, menu over viewer) with
      screenshot-based UI tests on both OSes.
- [ ] 5.3 If candidate 1 fails any placement or performance scenario on an OS, implement candidate 2, then candidate
      3, for that OS behind the same viewer interface; verify the same tests. Record each candidate's results.
- [ ] 5.4 Wire playback into the viewer with transport UI (buttons, J/K/L, arrow-key step, Home/End, scrub bar,
      timecode entry, loop) and the diagnostics overlay; verify every media-playback transport scenario in the built app
      on both OSes.

## 6. Gate evaluation (integration)

- [ ] 6.1 Run `ove-bench` and in-app playback for the full matrix on the macOS and Windows reference machines, with the
      UI idle and with a panel animating; record all numbers in design.md Results and compare to the
      media-playback performance requirement.
- [ ] 6.2 Measure the release build installer size and idle RAM with one file open on both OSes; record in Results.
- [ ] 6.3 Write the gate decision in design.md (chosen layering per OS, unmet targets, follow-ups) and present it to the
      owner for sign-off before changes 5–7 start.
