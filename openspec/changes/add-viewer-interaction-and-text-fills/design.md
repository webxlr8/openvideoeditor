# Design: add-viewer-interaction-and-text-fills

## Context

See proposal.md (Why). The viewer is a native surface under a transparent web view (`apps/desktop/src-tauri/src/viewer`).
On Windows the default is the stream fallback that draws into a canvas. Pointer events already reach the web view on
both platforms. Text is rasterised to RGBA and placed with an affine transform (`crates/ove-render/src/text.rs`,
`render.rs`). Captions already highlight the active word. Requirements: specs/viewer-interaction, specs/text-fills.

## Goals / Non-Goals

**Goals:**
- Hit testing that always matches what is rendered, because both come from one geometry source.
- One revision per gesture; live preview while dragging.
- Agents can read layer geometry and see guides in their frames.

**Non-Goals:**
- Native-drawn overlays. The overlay is web content, which works the same on every viewer backend.
- Editing keyframes on canvas.

## Decisions

### D1. Geometry in Rust, shared with the compositor
`ove-render::eval::layer_geometry(doc, t)` returns, per visible layer, its item id, its on-screen quad (four corners
after transform, tilt and placement), its crop quad and its stacking index. It uses the same `Layer` and transform
evaluation the compositor uses. Hit testing walks layers from the top and tests the point against the quad, then
against the layer's rendered alpha at that point. Alternative: compute geometry again in TypeScript (two
implementations, drift on rotation and crop).

### D2. Overlay as SVG over the viewer rectangle
Handles, guides and masks are SVG in the web view, positioned with the existing viewer rectangle. While dragging, the
UI sends a transient override to the preview engine, which renders the next frame with it. On pointer-up, one apply
op is sent, giving one revision. Handles show only while paused or scrubbing. Alternative: native overlay per platform
(three code paths, no gain).

### D3. Snapping
Move snaps to canvas centre and edges within 8 screen pixels. Edge snapping is off when the layer is rotated. Rotation
snaps to 15° steps with Shift. Crop drags are converted into the layer's own axes, so cropping a rotated layer behaves
as expected. Alternative: snap to other layers' edges (useful, but deferred to keep this change smaller).

### D4. Guides and the safe-area lint
Guides are view state, not document data, and are kept per project in UI state. Safe areas use 3.5% (action) and 5%
(title) insets. Platform UI-safe zones for 9:16 are editable presets because platforms change them. The verify lint
`text.outsideTitleSafe` uses `layer_geometry` for text and caption items. `ove look --overlay safe,grid,mask:9:16`
renders the same overlays into the agent frame only; export never includes them.

### D5. Full-screen viewer and playback rates
Full screen resizes the native surface to the window and hides the other panels. Floating controls hide after 2 s
without pointer movement. Esc exits. The transport accepts a rate command. Audio is resampled at rates up to 2× and
muted above that; pitch preservation comes later with a time-stretcher. Alternative: a second window (more native
surface plumbing for no gain).

### D6. Text fills
- `footage`: the text is rendered as a stencil. Inside the glyphs the composite below shows through; outside, the layer
  fills with `footageMatteColor` (default black) at the layer's opacity. Implemented as a stencil pass that reads the
  composite below.
- `inverted`: glyphs render white with the difference blend over the composite below.
Text effects and animations apply to the stencil. Alternative: footage fill as a general track matte feature (bigger
scope; a later change can generalise it).

### D7. Perspective tilt
`transform.rotationX` and `rotationY` tilt text layers. The four corners are projected with one focal length (at least
twice the canvas long edge) around the layer centre, then Z rotation applies. A new projective draw (inverse
homography with bilinear sampling) renders the quad on CPU and GPU. Hit testing uses the same quad. Tilt is limited
to ±80° on each axis so the quad never inverts.

### D8. Per-word spans
`textSpans` is a list of `{from, to, color, fontWeight, fontStyle, backgroundColor, scale}` with word indexes. Spans
combine with word animations: an animation changes timing and motion; a span changes style. When the text changes and
a span falls outside the word count, apply drops that span and `check` warns. Alternative: inline markup in the text
string (agents would have to escape text; merge would treat the text and style as one value).

### D9. Lottie through a vector rasteriser
A `lottie` item renders the requested frame directly from the Lottie JSON or `.lottie` archive through ThorVG's
software rasteriser into RGBA, cached by asset, frame and size. No transcode and no disk cache. Output is deterministic
and identical on both operating systems. Alternatives: bake to an alpha video file (disk churn, fixed resolution);
rlottie (mixed third-party licences, slow maintenance); a pure-Rust GPU renderer (promising, but immature for full
Lottie coverage today).

## Risks / Trade-offs

- [Overlay lags video during playback] → handles only while paused or scrubbing; detected by a UI test that checks
  handles are hidden during playback.
- [Drag preview is slow on the CPU backend] → transient overrides render at preview resolution only; detected by
  measured drag-to-frame latency in the diagnostics overlay.
- [ThorVG lacks some Lottie features (expressions, some mattes)] → corpus test and `check` diagnostics; detected by the
  corpus SSIM test.
- [ThorVG C++ build fails on one platform] → spike task first; detected by CI builds on both operating systems.
- [Word indexes shift after a text edit] → apply drops out-of-range spans and `check` warns; detected by a projection
  test.

## Migration Plan

All new fields are optional. Old documents print and render exactly as before. The `lottie` item type is new; older app
versions would see it as an unknown item type, which the projection already reports.

## Open Questions

- Waves proceed in the master-plan order (default until the owner decides otherwise).
- Default platform UI-safe zone presets for 9:16: our own measured values, to be reviewed by the owner.
- Lottie corpus: licence-checked files only; the owner may supply production samples.

## Results

Measured on Windows 11 x64 (32 cores, `CARGO_BUILD_JOBS=8`), unless a line says otherwise.

### Task 5.1: Lottie spike (Windows x64 only)

- ThorVG builds through dotlottie-rs 0.1.61 (MIT; ThorVG MIT, its PNG and JPEG loaders MIT) with MSVC and the
  LLVM `libclang` that bindgen already needs for speech recognition. No extra tools: dotlottie-rs compiles ThorVG with
  the `cc` crate, so meson and ninja are not needed. Nothing is downloaded at build time for the software rasteriser
  (only the WebGPU backend, not enabled, would download a prebuilt library).
- Features used: `tvg`, `tvg-cpu`, `tvg-png`, `tvg-jpg`, `dotlottie`. Expressions (`tvg-lottie-expressions`) and
  ThorVG's worker threads are off: expressions are reported as unsupported by design, and one thread keeps frames
  deterministic.
- It is behind the `lottie` feature of `ove-graphics`, off by default, so no shipped binary changes yet. The renderer
  is `crates/ove-graphics/src/lottie.rs`; its tests cover the requested frame, sharp edges at 4× size, byte-identical
  repeat renders and the refusal of corrupt data (4 tests, all pass).
- Lock file: bindgen 0.73.2 (pulled in by dotlottie-rs) accepts syn 2 or 3, but the resolver gave it syn 3 next to
  prettyplease 0.2, which only works with syn 2, and bindgen failed to compile. The lock file pins that bindgen to
  syn 2.
- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok.
- Build time of dotlottie-rs with ThorVG from clean (debug): 18 s. Release build of the example below with all of
  `ove-graphics`: 3 min 44 s.
- Binary size: the release example `lottie_frame` (Rust standard library, dotlottie-rs and ThorVG; the rest of
  `ove-graphics` is dropped by the linker) is 857,600 bytes, so ThorVG adds about 0.7 MB.
- One 1920×1080 frame of a small test animation, including parsing and the buffer: 9.8 ms (frame 0) and 16 ms
  (frames 12 and 29), release build. The ≤ 4 ms target of task 5.2 is for a cached animation on M1 and is not measured
  here.
- Open: the macOS arm64 build and CI on both systems (task 5.1 stays open until they run).
