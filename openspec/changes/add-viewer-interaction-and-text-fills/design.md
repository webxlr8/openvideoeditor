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

### Group 1: layer geometry and hit testing

- `crates/ove-render/src/geometry.rs` (re-exported from `ove-render::eval`): per visible layer the item id and type,
  stacking index, the source quad, the crop quad, rotation and opacity. It uses `layers_at` and the placement the
  renderer draws with: `media_placement` for clips, and the new `TextFrame`, `layer_area`, `code_size` and
  `code_place` in `render.rs`, which `draw_layer` now uses too, so drawing and geometry share one code path. Captions
  are measured by drawing them alone and taking the bounds of their pixels. Transition motion is not part of the
  geometry (the resting place is reported).
- Property test (`crates/ove-render/tests/geometry.rs`): random transforms, rotations, mirrors and crops of a video, a
  still and a boxed title, each layer rendered alone at 480×270 and sampled every 2 pixels. With 256 cases (the
  per-group count): 100.0000 % of 24,883,200 points agree (crop quad holds the point ⇔ rendered alpha > 0); the
  target is 99.5 %. Tilted fixtures wait for `transform.rotationX/Y` (task 4.1).
- Hit testing (`geometry::hit_test`): topmost selectable layer whose crop quad holds the point and whose own rendered
  pixel there has alpha > 0. The transparent-title scenario passes in Rust (gap between the letters of "I I" selects
  the clip below) and in the UI suite.
- `ove geometry <project> --at <s> [--height <px>]` and the `layer_geometry` tool (in-app, MCP, pinned). Past the
  end: exit 1, "60 s is past the end of the timeline, which is 4 s long". `docs/cli/geometry.md` examples run in
  `crates/ove-cli/tests/docs.rs` (with `--features media`).

### Group 2: handles

- The overlay is SVG in the web view over the viewer rectangle (`apps/desktop/ui/src/overlay/`), the same on every
  viewer backend. Move (snaps the centre to the canvas centre, and the edges to the canvas edges when the layer is
  not rotated, within 8 px), scale from the corners, rotate (15° steps with Shift), and crop along the layer's own
  edges (video, stills and code items; text and captions draw without a crop, captions without rotation).
- A drag sends transient previews (`viewer_preview`: the engine swaps the document in the running timeline renderer
  and re-renders the paused frame, no revision; one preview in flight at a time) and one `update_item` on release.
  Esc puts the layer back with a preview of the saved document. Handles hide while playing.
- UI tests: one revision per drag, undo, snapping, Esc cancel, scale, Shift rotation, crop of a layer rotated 30°,
  no crop handles on titles, hidden during playback.

### Group 3: guides, lint, full screen and rates

- Guides (thirds, 2–5 grids, action safe 93 %, title safe 90 %, centre cross, masks 2.39:1, 1.85:1, 1:1, 9:16, 4:5,
  editable platform zones) are per-project view state in the web view's storage; nothing reaches the document.
  Starting zones ("Vertical feed", "Vertical stories") are our own values for the owner to review.
- `ove look --overlay safe,grid,grid:<n>,thirds,centre,mask:<w:h>` draws the same guides into agent frames
  (`crates/ove-analyze/src/guides.rs`); three golden overlay images. An exported H.264 frame matches the frame without
  guides within 12/255 at five guide pixels while the agent frame shows them.
- Note: the older `look --grid` keeps its own boxes (title safe 80 %, action safe 90 %); `--overlay safe` and the
  lint use this change's 90 % and 93 %.
- `text.outsideTitleSafe` (verify warning, registered by `ove-tools` when it has the renderer): samples each text and
  caption item every 0.25 s (at most 40 times) and at each caption segment's middle, and names the item, the time and
  the edge. Fixtures: a title at 98 % of the width is reported at 1 s; a centred title and a default caption are not;
  a caption moved to the bottom edge is.
- Full screen: the viewer fills the window (and the window goes full screen), floating play, scrub, timecode, loop,
  rate and guides controls hide after 2 s without pointer movement, Esc returns.
- Rates 0.5×, 1×, 1.25×, 1.5×, 2× (`ove-playback`: `Transport.rate`, `Command::SetRate`, `supported_rate`). Audio
  keeps mastering the clock at every forward rate: the engine mixes `rate` times as much timeline audio per block and
  resamples it (linear, pitch follows the rate); the ring counts media time at the rate. Simulated: 15.0 s of
  timeline in 10 s of wall time at 1.5× on the system clock, and 12 s in 8 s on the audio clock, both within one
  frame. 7× is refused in the app command, over the control link (`rate.unsupported`) and in `ove app play --rate`.
