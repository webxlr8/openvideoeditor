# Design: add-desktop-shell-and-native-preview

## Context

See proposal.md (Why). The repository has no code yet. Decisions that frame this change are in
docs/03-architecture-plan.md §2–3.3. Requirements are in specs/desktop-shell and specs/media-playback.

## Goals / Non-Goals

**Goals:**
- Decide R1 by measurement: which viewer layering works on macOS and Windows 11 at the required performance.
- Leave behind production-grade foundations: workspace layout, bridge, logging, CI, FFmpeg build, decode/playback core.

**Non-Goals:**
- Compositing more than one layer, project documents, timeline UI, export (later changes).
- Visual design of the real UI (change 7); the UI here is a structural placeholder.

## Decisions

### D1. Workspace layout
```
crates/ove-core        (empty in this change; document model arrives in change 2)
crates/ove-media       probe, demux, decode (hw + sw), frame pool, audio decode/resample
crates/ove-playback    clock, scheduler, A/V sync, transport state machine, diagnostics
crates/ove-gpu         wgpu device/surface management, frame upload, viewer renderer, device-loss recovery
crates/ove-bench       headless benchmark binary (no UI)
apps/desktop           Tauri app (Rust) + ui/ (React 19, TypeScript, Vite)
tools/ffmpeg           reproducible LGPL FFmpeg builds for macOS arm64 and Windows x64
```
Rationale: media/playback/gpu stay UI-free so the benchmark and later CLI reuse them. Alternative: one crate — rejected,
it couples the headless tools to Tauri.

### D2. Viewer layering — candidates measured in this order
1. **Under + transparent web view (preferred).** macOS: an NSView with a CAMetalLayer inserted below the WKWebView in
   the window's content view; web view background transparent. Windows: wgpu surface for the Tauri host window (or a
   DirectComposition visual) with WebView2 `DefaultBackgroundColor` transparent; the viewer rectangle in the DOM is
   transparent, everything else opaque. Web popups draw over video naturally.
2. **Above (child surface) + popup layer.** Native child view/HWND above the web view, sized to the viewer rectangle.
   Web popups that intersect the viewer render in a second small transparent web view placed above it.
3. **Frame streaming fallback.** Rust renders into a shared-memory buffer; the UI paints it on a canvas
   (custom-protocol or shared buffer). Simplest layering, highest latency and CPU cost.
Pass criteria per candidate: all desktop-shell placement scenarios pass, and media-playback performance targets are met
with the UI idle and with the UI animating a panel. Choose the first candidate that passes on both OSes; macOS and
Windows may use different candidates behind one viewer interface. Alternatives rejected outright: drawing video inside
the web view with WebCodecs/<video> (no ProRes/DNxHR/MXF support in system web views; different codecs per OS).

### D3. Viewer rectangle sync
The UI measures the viewer element (ResizeObserver + window resize/scale events) and sends rect + scale factor over the
bridge; the native side applies it on the next frame. During live window resize on macOS, layer resizing is done inside
the same CATransaction as the web view's layout to avoid tearing; on Windows the swap chain is resized with
`ResizeBuffers` on the render thread. Alternative: native side computes layout itself — rejected, duplicates CSS layout.

### D4. Decode pipeline
FFmpeg 9.x via ffmpeg-the-third, LGPL configuration (`--disable-gpl --disable-nonfree`, shared libs), only the demuxers,
decoders, parsers and hwaccels we need, to keep size down. Hardware: VideoToolbox on macOS; D3D11VA on Windows
(Media Foundation is not needed for decode; D3D11VA covers NVIDIA/AMD/Intel). Decoded hardware frames are imported into
wgpu without a CPU copy where the platform allows (IOSurface → Metal texture; D3D11 texture shared to D3D12); otherwise
one upload copy. A decode thread per open file feeds a bounded frame queue (default 8 frames 4K, configurable);
seeks flush the queue, seek to the previous keyframe and decode forward to the target PTS, discarding output.
Alternatives: platform-native decoders only (AVFoundation/MF) — rejected, no MXF/DNxHR on Windows and two code paths;
GStreamer — rejected, heavier distribution on Windows.

### D5. Clock and sync
Audio clock is master while audio plays (cpal output callback position, corrected for device latency); system
monotonic clock otherwise. Video frames are presented when their PTS ≤ clock, dropped when late by more than one frame
interval. Shuttle speeds other than 1× play video only, audio muted (pitch-correct scrubbing is out of scope here).

### D6. Bridge
Tauri commands/events with types exported to TypeScript from Rust definitions (tauri-specta). Generated bindings are
committed and CI fails if regeneration produces a diff. All commands return `Result<T, AppError{code, message}>`.

### D7. Logging and crash containment
`tracing` with a rotating file appender in the platform log dir; panics in worker threads are caught at thread
boundaries and turned into `AppError`. Crash reporting service integration waits for change 16.

### D8. FFmpeg build and licence enforcement
`tools/ffmpeg` builds pinned FFmpeg sources in CI per target, records configure flags and checksums, and fails if
`--enable-gpl` or any GPL/nonfree component appears. cargo-deny enforces the allowed licence list for Rust crates; a
license-checker step enforces it for JS.

## Risks / Trade-offs

- [WebView2 transparency over a sibling GPU surface may not composite correctly, or flicker on resize] → candidates 2
  and 3 are planned and measured; detected by the placement scenarios and a resize-stress UI test with screenshots.
- [Zero-copy hardware frame import not available for some formats/drivers] → one-copy upload path; detected by the
  diagnostics overlay (reports copy count) and the performance targets.
- [HEVC 4:2:2 10-bit not hardware-decodable on many Windows GPUs] → software path; detect with the media matrix; if
  4K 4:2:2 software decode misses targets, document it and rely on proxies (change 5).
- [FFmpeg libraries push installer over budget] → minimal configure; measure installer size in CI from the first
  build.
- [System web view differences (WebKit vs Chromium) break UI layout] → UI tests run on both OSes from this change on.
- [Display sleep / GPU reset on Windows] → device-loss recovery path tested by forcing device removal (D3D12 debug
  layer `RemoveDevice`).

## Migration Plan

None (first change).

## Open Questions

- Exact reference machines for performance numbers. Local development machine is a MacBook Air M4 16 GB, which is
  faster than the minimum we want to support; the gate is measured on it with the M1 8 GB targets tightened by 30%
  unless an M1-class Mac is provided. The Windows reference laptop (and a second GPU vendor) must be provided by the
  owner before the gate is evaluated. This does not change the specs or tasks.
- Local disk has ~14 GB free; the Rust toolchain, FFmpeg builds and target directories need ~15–20 GB. Free space or
  an external build volume is needed before task group 1.
