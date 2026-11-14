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

## Results (Linux dev container, partial)

Measured 2026-11-11 on the cloud development container: Linux x86-64, 4 vCPU Intel Xeon @ 2.10 GHz, 15 GB RAM, **no
GPU, no audio device**. This is neither reference machine; the numbers below show the headless pipeline works and give
a software-only baseline. They do not decide the R1 gate (tasks 6.1–6.3).

### Toolchain (task 1.1)
`tools/check-toolchain.sh` passes: rustc/cargo 1.97.0 (`rust-toolchain.toml`), Node 22.23.4 (`.node-version`),
pnpm 10.28.0 (`.pnpm-version`).

### FFmpeg build (task 1.4, Linux only)
`tools/ffmpeg/build.sh`: FFmpeg 9.0.2 (sha256 pinned), LGPL 2.1+, shared, `--disable-everything` plus the demuxers,
decoders and parsers for the formats in specs/media-playback; dav1d for AV1. Licence gate verified both ways: passes on
our configuration, fails on the distribution FFmpeg (`--enable-gpl --enable-libx264 …`) and on a nonfree sample.
Shipped libraries total **10.3 MB** (avcodec 6.8, swscale 2.1, avutil 1.0, avformat 0.7, swresample 0.1; dav1d not
counted). Build time 1 min 47 s. The same test suite passes against this build and against the system FFmpeg 6.1.
Not done: macOS arm64 (VideoToolbox) and Windows x64 (D3D11VA, MSVC) builds and the CI artefact upload.

### Media matrix (task 2.1, synthetic stand-ins only)
`tests/media/media.py` generates 20 files with sha256 in `tests/media/MANIFEST.json` (regeneration is byte-identical
with the same ffmpeg build): H.264 long-GOP (+AAC), H.264 60p, H.264 3840×2160 60p, HEVC Main10 4:2:0, HEVC 4:2:2
10-bit, HEVC Main10 HDR (BT.2020/PQ) VFR (+AAC), ProRes 422 Proxy/LT/HQ (+PCM 24) and 4444 with alpha, DNxHR HQ in
MXF (+PCM), AV1 (MKV), VP9 + Opus (WebM), H.264 VFR (+AAC), AAC/PCM/ALAC/Opus audio-only, two truncated files. The
`fetch` hook downloads and checksums owner-provided camera originals listed under `real` (none yet). Task 2.1 stays open
until real footage (Apple ProRes, iPhone HDR, camera MXF, 1-hour long-GOP file) is in the manifest.

### Probe, decode, seek, audio (tasks 2.2, 2.3, 2.5)
- Probe matches `ffprobe` (container, duration ±1 ms, codec, profile, size, pixel format, avg/base frame rate, frame
  count, audio codec/rate/channels/layout/sample format) for all 19 openable files; VFR flagged for exactly the two VFR
  files. Probe time: 0.2–19.5 ms (largest: 20 MB MXF).
- Linear decode: per-frame md5 equals `ffmpeg -f framemd5` for all 14 video files.
- **200 random seek targets per file** (half on a frame's pts, half inside its interval): all frame-exact, including
  the frame after the target. Worst seek-to-frame (release, FFmpeg 9.0.2): ProRes 5–8 ms, DNxHR 720p 94 ms, H.264
  180p 33–53 ms, H.264 4K60 246 ms, HEVC HDR 720p 163 ms.
- Audio decode + resample (48 and 44.1 kHz, S16 and F32): **0 LSB** difference from the same-version ffmpeg CLI for all
  8 audio-bearing files; sample-accurate seek (first chunk within one sample of the target).
- Mid-stream hardware failure (fault-injected fake hardware decoder failing at frame 37): playback continues in software
  with every frame present exactly once and md5-identical (H.264, HEVC 10-bit, VFR); failure during seek also falls
  back. Deleted file → `media.offline` within 32 packet reads; truncated file ends cleanly; missing moov → `media.corrupt`.

### Playback engine (task 3.1)
36 unit/scenario tests on simulated time: every transport transition (play/pause/toggle, J/K/L −2×…2×, step ±1 incl.
"10 steps = 10 frames", start/end, seek while playing, loop, play-at-end restart, reverse to start), late-frame dropping,
VFR presentation at each frame's own timestamp, audio-master clock with 25 ms device latency (A/V offset ≤ 2 frames),
audio underrun holds video, no-device system clock, error stops playback with a message.

### Benchmark (task 3.3, Linux only)
`ove-bench` (release, FFmpeg 9.0.2 LGPL build, software decode, 8-frame queue, 1 kHz scheduler tick, 100 seeks).
Every report validates against `crates/ove-bench/schema.json`.

| file | codec / pix_fmt | size | fps | played s | presented | dropped | A/V offset mean/p95/max (µs) | seek p50/p95/max (ms) | exact | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|---|
| av1.mkv | av1 yuv420p | 320×180 | 30 | 3.00 | 90 | 0 | 273 / 571 / 627 | 10.2 / 25.1 / 31.4 | yes | 12 |
| dnxhr_hq.mxf | dnxhd yuv422p | 1280×720 | 25 | 2.00 | 50 | 0 | 330 / 582 / 597 | 18.7 / 40.8 / 49.3 | yes | 65 |
| h264_4k60.mp4 | h264 yuv420p | 3840×2160 | 60 | 1.00 | 60 | 0 | 289 / 549 / 592 | 98.4 / 154.2 / 243.8 | yes | 276 |
| h264_60fps.mov | h264 yuv420p | 320×180 | 60 | 2.00 | 120 | 0 | 292 / 565 / 1272 | 7.1 / 13.5 / 19.3 | yes | 13 |
| h264_longgop.mp4 | h264 yuv420p | 320×180 | 30 | 4.00 | 120 | 0 | 446 / 638 / 9804 | 5.6 / 10.3 / 23.3 | yes | 18 |
| hevc_hdr_vfr.mov | hevc yuv420p10le | 1280×720 | 29.3 VFR | 1.99 | 59 | 0 | 277 / 540 / 579 | 51.0 / 87.7 / 136.2 | yes | 76 |
| hevc_main10_420.mp4 | hevc yuv420p10le | 320×180 | 30 | 3.00 | 90 | 0 | 303 / 609 / 648 | 13.3 / 20.8 / 22.5 | yes | 12 |
| hevc_main422_10.mov | hevc yuv422p10le | 320×180 | 30 | 3.00 | 90 | 0 | 317 / 599 / 685 | 15.7 / 25.5 / 37.4 | yes | 13 |
| prores_4444.mov | prores yuva444p12le | 320×180 | 30 | 3.00 | 90 | 0 | 291 / 556 / 619 | 3.2 / 5.1 / 6.5 | yes | 13 |
| prores_hq.mov | prores yuv422p10le | 320×180 | 30 | 3.00 | 90 | 0 | 310 / 569 / 625 | 1.7 / 2.8 / 5.9 | yes | 17 |
| prores_lt.mov | prores yuv422p10le | 320×180 | 30 | 3.00 | 90 | 0 | 308 / 578 / 676 | 1.3 / 2.9 / 4.1 | yes | 10 |
| prores_proxy.mov | prores yuv422p10le | 320×180 | 30 | 3.00 | 90 | 0 | 325 / 587 / 660 | 0.7 / 1.5 / 3.9 | yes | 10 |
| vfr_phone.mp4 | h264 yuv420p | 320×180 | 29.9 VFR | 3.98 | 119 | 0 | 315 / 581 / 690 | 9.2 / 16.3 / 21.2 | yes | 20 |
| vp9_opus.webm | vp9 yuv420p | 320×180 | 30 | 3.00 | 90 | 0 | 296 / 583 / 1083 | 11.8 / 25.6 / 28.7 | yes | 14 |
| h264 4K30, 10 s, GOP 60 (not in matrix) | h264 yuv420p | 3840×2160 | 30 | 10.00 | 300 | 0 | 310 / 562 / 654 | 230.6 / 490.3 / 671.3 | yes | 390 |
| ProRes 422 HQ 4K30, 5 s (not in matrix) | prores yuv422p10le | 3840×2160 | 30 | 5.00 | 150 | 0 | 370 / 966 / 3653 | 136.1 / 169.1 / 198.7 | yes | 466 |

Observations:
- 4K30 H.264, 4K60 H.264 (1 s) and 4K30 ProRes 422 HQ play at 1× with **0 dropped frames in software** on 4 vCPU.
  This needed the YUV repack path (`ove-media` convert.rs): the first version converted 4:2:2/4:4:4 to RGBA with
  swscale and dropped 89 of 91 frames at 4K; repacking planar YUV straight to NV12/P010 (4:2:2/4:4:4 chroma subsampled
  for preview only) removed the bottleneck.
- Seek p95 for long-GOP 4K H.264 in software (490 ms, 2 s GOP) is above the 300 ms target; ProRes 4K (169 ms) is above
  150 ms. Both targets are defined for the reference machines with hardware decode; re-measure there (6.1).
- Peak memory at 4K is 390–466 MB with the 8-frame queue (8 × 24 MB P010 frames + decoder pools). This exceeds the
  250 MB idle budget only during playback; the queue depth is configurable and should be re-tuned with zero-copy
  hardware frames on the reference machines.

### Not verified here (and why)
- 1.4: macOS/Windows FFmpeg builds, CI artefacts — need the macOS/Windows runners.
- 2.1: real camera footage — owner provides it (docs/03 §6).
- 2.4: hardware decode on VideoToolbox / D3D11VA and the checksum suite on the hardware path — this container has no
  GPU; only the fallback logic (fault injection) and "no hardware available → software" are tested.
- 3.2: output-device switching (manual, both OSes); the no-device path is verified for real here (ALSA reports no
  device → `AudioUnavailable`, player on the system clock) and the rebuild decision is unit-tested.
- 3.3: running `ove-bench` on the CI OSes; done on Linux for every matrix file.

### Integration: decode + playback in the desktop app (Linux dev container)

The shell's interim test-pattern transport was replaced by `apps/desktop/src-tauri/src/engine.rs`: a playback thread
that opens files with `ove_media::open_video` (hardware first, software fallback) behind a `DecodeWorker`, drives them
with `ove_playback::Player` (audio-master clock when an output device exists, system clock otherwise), feeds the audio
ring from `AudioDecoder`, and sends decoded NV12/P010/RGBA frames to the GPU viewer. The bridge contract
(`TransportCommand`, `PlaybackState`, `MediaInfo`) is unchanged except that `open_media` is now async.

Verified in the real app under Xvfb (software GPU, no audio device) with a 1280×720 25 fps H.264 + AAC file whose
frames carry a burned-in frame counter:
- The file opens through the decoder (`h264 (High)`), its first frame shows, and playback advances; with no audio
  device the log says so and video runs on the system clock.
- **Frame step:** Home, then 10 rapid step-forward key presses → the viewer shows frame 10 (burned-in counter) and
  the UI timecode reads `00:00:00:10`.
- Two bugs found and fixed by this run: (1) the transport command returned the state from *before* the engine
  applied it and the UI could apply that reply over a newer `playbackChanged` event (timecode one frame behind the
  picture) — events are now authoritative and replies only carry errors; (2) a step pressed while the previous step's
  frame was still decoding was merged into it — the engine now queues steps and issues each one when the player has
  settled (`Player::is_settled`). Plus a frame-index rounding fix (2.88 s × 25 fps is frame 72, not 71), unit-tested
  at 24/25/30/60 and 29.97 fps.

Still needed on macOS/Windows (tasks 5.x, 6.x): native layering in place of the Linux frame-stream fallback,
hardware decode, audio device switching, and the performance gate.

### UI reference check

The Home and Editor placeholders were compared side by side with the reference screenshots (01-home, 20-editor-default,
29b timeline hidden, 27a context menu). Layout matches: left navigation with search and bottom items; centred prompt
card with chips and a templates/projects tray; editor icon rail with seven panels, collapsible side panel, viewer,
transport bar, four-track timeline (Text, Captions, Overlay, Main), assistant panel with composer, and the
hidden-timeline mode with a scrub bar and timecodes. All copy, icons and branding are our own. Differences are later
scope: the properties panel and per-item context menus, timeline zoom and snapshot controls (change 7).
