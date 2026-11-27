# Tasks: add-viewer-interaction-and-text-fills

## 1. Layer geometry and hit testing

- [ ] 1.1 Implement `layer_geometry(doc, t)` in `ove-render::eval` from the compositor's layer data; verify the
      property test (quad contains point ⇔ rendered alpha > 0 for ≥ 99.5% of sampled points) on rotated, cropped and
      tilted fixtures. (Rotated and cropped fixtures pass; the tilted ones wait for `transform.rotationX/Y` in 4.1.)
- [x] 1.2 Add `ove geometry --at` and the MCP tool, documented in `docs/cli/`; verify CLI tests for success and the
      time-past-end refusal, and that the docs examples run.
- [x] 1.3 Viewer click-to-select using geometry plus alpha; verify a UI test for the transparent-title scenario.

## 2. Handles

- [ ] 2.1 SVG overlay layer over the viewer rectangle on every viewer backend; verify it lines up with the frame
      within 1 screen pixel on macOS and on the Windows stream backend.
- [x] 2.2 Move, scale, rotate and crop handles with transient preview overrides, snapping, Esc cancel, and one apply op
      on release; verify UI tests: one revision per drag, undo, cancel, rotated crop, hidden during playback.
- [ ] 2.3 Tilt handles for text; verify a UI test that one tilt drag creates one revision. (Waits for
      `transform.rotationX/Y`, task 4.1.)

## 3. Guides, lint, full screen and rates

- [x] 3.1 Grids, safe areas, centre cross, format masks and editable platform presets as UI view state; verify guides
      never appear in an export (golden export test).
- [x] 3.2 `ove look --overlay` and the verify lint `text.outsideTitleSafe`; verify golden overlay images and lint
      fixtures for both scenarios.
- [ ] 3.3 Full-screen viewer with floating controls and Esc exit; verify a UI test on both operating systems.
- [x] 3.4 Transport rate command (0.5×–2×) with resampled audio and control-link access; verify the 1.5× timing test
      and the unsupported-rate refusal.

## 4. Text fills, tilt and spans

- [ ] 4.1 Add `textFill`, `footageMatteColor`, `transform.rotationX`, `transform.rotationY` and `textSpans` to the
      schema, projection and `timeline.d.ts`; verify projection round-trip tests and range diagnostics.
- [ ] 4.2 Footage fill (stencil pass) and inverted fill (difference blend) on CPU and GPU; verify golden frames for
      both fills and the nothing-below scenario.
- [ ] 4.3 Projective draw on CPU and GPU and tilt rendering; verify corners within 0.5 px of the analytic result and
      quad-to-alpha IoU ≥ 0.98.
- [ ] 4.4 Per-word spans combined with word animations, with out-of-range drop and `check` warning; verify golden
      frames and the shortened-text scenario.
- [ ] 4.5 Document the new attributes in `docs/spec/projection-format.md`, `docs/spec/timeline.d.ts` and `ove docs`;
      verify the typings drift check passes.

## 5. Lottie

- [ ] 5.1 Spike: build ThorVG through dotlottie-rs on macOS arm64 and Windows x64 with cargo-deny passing; verify CI
      builds on both and record build time and binary size in Results. (Windows x64 built, measured and passing
      `cargo deny`; macOS arm64 and CI are open.)
- [ ] 5.2 `lottie` item type in `ove-graphics` with direct frame rendering, cache by asset, frame and size, `speed`,
      `loop`, `holdLast` and colour slots; verify byte-identical repeat renders and ≤ 4 ms per 1080p frame on M1.
- [ ] 5.3 Lottie import and `check` diagnostics for unsupported features and corrupt files; verify the licence-checked
      corpus scores SSIM ≥ 0.98 against reference PNGs and the corrupt-file refusal.
- [ ] 5.4 Mark add-motion-graphics task 1.5 as covered by this change; verify the task list references this change.

## 6. Integration checks

- [ ] 6.1 Built app on macOS and Windows: select by click, move, rotate, crop and tilt a title with handles, turn on
      guides, play full screen at 1.25×, import and play a Lottie file, export; verify one revision per gesture, no
      guides in the export, and record drag latency in design.md Results.
