# Results: desktop shell, bridge and native viewer (task groups 4 and 5)

Recorded 2026-11-11 on the Linux development container (x86_64, no GPU; Mesa llvmpipe Vulkan
software adapter; Xvfb for the real app). macOS and Windows have **not** been run yet: everything
platform-specific below is marked UNVERIFIED. Numbers from this machine are not gate numbers.

## What exists

| Area | Where | State |
|---|---|---|
| GPU renderer: NV12 / P010 / RGBA8 upload, YUV→RGB (BT.601/709/2020, limited/full), letterboxing, offscreen readback, device-loss recovery | `crates/ove-gpu` | Tested headless (llvmpipe) |
| Zero-copy import hooks (IOSurface, D3D11 shared handle) | `crates/ove-gpu/src/import.rs` | Interface only; returns `Unsupported`, upload path used (`copiesLastFrame = 1`) |
| Tauri 2.12 app, dark window, 1440×900 default, 1024×640 minimum, hidden-inset title bar (macOS config), window-state restore with clamping | `apps/desktop/src-tauri` | Clamping unit-tested; window placement observed under Xvfb |
| Typed bridge (tauri-specta), `AppError{code,message}`, committed `ui/src/bindings.ts`, `export-bindings` bin with `--check` | `src/lib.rs`, `src/commands.rs`, `src/bin/export-bindings.rs` | Tested |
| Single instance, argv / "Open with" (macOS `RunEvent::Opened`) / window drop routing, file associations | `src/lib.rs`, `tauri.conf.json` | Linux real-app run verified (below); macOS/Windows UNVERIFIED |
| Rotating logs (daily, 7 files) with startup line; panic hook with backtrace; contained workers | `src/logging.rs`, `src/worker.rs` | Tested |
| Viewer layering behind `NativeLayer`: candidate 1/2 macOS (CAMetalLayer NSView below/above WKWebView), Windows (child HWND bottom/top of z-order under transparent WebView2); candidate 3 frame streaming (`ove-frame://` URI scheme → canvas) | `src/viewer/` | Candidate 3 verified on Linux; macOS/Windows compile + clippy clean only (cross-target `cargo clippy`), UNVERIFIED at runtime |
| Interim transport (play/pause, step, J/K/L −2…2×, Home/End, loop, seek, volume/mute) driving a generated NV12 test pattern | `src/transport.rs`, `src/media.rs` | Tested; to be replaced by ove-playback + ove-media |
| UI: Home and Editor structural placeholders (rail with 7 panels, collapsible panel, viewer, transport, timeline, assistant panel, toggles), timecode entry, keyboard map, diagnostics overlay (Ctrl/Cmd+Shift+D) with GPU-loss / panic hooks, menus over the viewer | `apps/desktop/ui` | vitest + Playwright (browser, mocked bridge) |

Layering is selected with `OVE_VIEWER_LAYERING=below|above|stream` (default `below` on macOS and
Windows, `stream` elsewhere) so each candidate can be measured on the same build.

## Bridge

Commands: `appInfo`, `takePendingOpenPaths`, `openMedia(path)`, `openMediaDialog`, `closeMedia`,
`setViewerRect(rect)`, `transport(command)`, `setVolume(volume, muted)`, `playbackState`,
`diagnostics`, `simulateDeviceLoss`, `debugPanicWorker`, `debugPanicViewer`.
Events: `playbackChanged`, `openPathsRequested`, `viewerFrameReady`, `viewerBackendChanged`,
`appErrorRaised`. Error mode: every command resolves to `{status: "ok", data} | {status: "error",
error: AppError}`.

## Measurements (this container, not the reference machines)

| What | Value | Notes |
|---|---|---|
| GPU device-loss recovery (destroy device → recreate, re-upload, re-render) | 36 ms | `ove-gpu` test `recovers_from_device_loss`, llvmpipe; spec bound 2 s |
| Golden frames NV12 / P010 / RGBA (320×240, 16:9 letterboxed) | pass | max channel diff ≤ 6, ≤ 0.1 % of channels > 2; bars within 3 of CPU reference |
| Process start → window shown (debug build, Xvfb) | 864 ms first run, 205 ms second run | Not a cold-start measurement; task 4.5 needs release builds on the reference machines |
| Real app: second launch with a path | exits 0; path routed to the first instance; first instance shows “notes.txt can't be opened: .txt files are not supported …”; open file keeps playing | needs a D-Bus session bus on Linux (`dbus-run-session`) |

## Tests run

- `cargo test -p ove-gpu -p ove-desktop`: 53 tests pass (window placement incl. first launch,
  HiDPI, restore, disconnected display, off-screen, minimum size; transport incl. 10× step,
  J/K/L, loop/stop at ends; media open errors; stream presenter follows rect, survives a viewer
  panic, recovers device loss, stops when hidden; bindings current and stale-binding detection;
  worker panic → `AppError` and log contains message + backtrace).
- `cargo clippy -p ove-gpu -p ove-desktop --all-targets -- -D warnings` clean for
  x86_64-unknown-linux-gnu, aarch64-apple-darwin (`CC_aarch64_apple_darwin=clang`) and
  x86_64-pc-windows-msvc. `cargo fmt --check` clean.
- `pnpm run typecheck`, `pnpm run lint`, `pnpm run test` (15 vitest tests), `pnpm run licenses`
  (217 packages, 4 runtime; build-only exceptions documented in `scripts/check-licenses.mjs`).
- `pnpm e2e` (Playwright + Chromium against Vite with the mocked bridge): 18 checks pass — the
  rectangle sent over the bridge equals the viewer element's rectangle after first layout, window
  resize, side-panel collapse (viewer moves without resizing), timeline hidden, assistant hidden/
  shown, viewer collapsed (`visible: false`) and restored, at scale factor 2 and 1; zoom dropdown
  and right-click menu overlap the viewer and are topmost by `elementFromPoint`; diagnostics
  overlay toggles.
- Real app under Xvfb (`scratchpad/fork-desktop/run-app.sh`): window 1440×900 centred on a
  1600×1000 screen; file passed on the command line opens; GPU (llvmpipe) renders the test pattern
  and it is streamed into the viewer canvas; second launch routes to the first instance; log file
  written to `<data>/dev.openvideoeditor.app/logs/ove.<date>.log` with the startup line.

## Not verified / open

- macOS and Windows: nothing has run. Candidate 1 on Windows (child HWND under a transparent
  WebView2) is the least certain piece; candidate 2 is one env var away; the DirectComposition
  variant is not implemented. macOS CATransaction-synchronised resizing (design D3) is not done:
  the layer frame is set on the main thread right after the UI reports the rectangle.
- Zero-copy import (IOSurface / D3D11) is not implemented.
- Cold start, idle RAM, installer size, dropped frames, A/V offset: need ove-media/ove-playback
  wiring and the reference machines (tasks 4.5, 5.4, 6.x).
- Window-state save happens on window close (`CloseRequested`), not on a killed process.
- The `ove-frame` stream path caps frames at 1920×1080 and copies RGBA through the web view; it is
  a development/fallback path, not the gate candidate.
- The UI is a structural placeholder modelled on the reference layout (no copied copy, brand,
  icons or fonts); the real UI is change 7.

## Task checkboxes

- 4.2 ticked: structured errors reach the UI (vitest + real-app toast) and the drift check fails a
  deliberately stale binding (`stale_bindings_are_detected`, `export-bindings --check`).
- 4.4 left open: logs, startup line and panic containment are done and tested, but the startup
  line's decode capabilities are a placeholder until ove-media is wired in.
- 4.1, 4.3, 4.5, 5.1–5.4 left open: their verification requires macOS and Windows (5.1 also a
  forced device removal on Windows; the headless golden and device-loss tests pass here).
