# Proposal: add-viewer-interaction-and-text-fills

## Why

People cannot move, scale, rotate or crop a layer by dragging it in the viewer. They must type numbers in the
inspector. There are no guides, safe areas or format masks, so titles end up outside safe areas without warning. Text
can only be filled with a colour. Lottie animations, a common motion-graphics format, cannot be imported. Agents have
no way to read where layers are on screen, so they cannot check framing or overlaps without guessing from pixels.

## What Changes

- Layer geometry from the renderer: for any time, the on-screen quad, crop quad and stacking order of every visible
  layer, computed from the same data the compositor draws. `ove geometry --at <t>` and an MCP tool expose it to
  agents.
- Hit testing in the viewer from that geometry. A click selects the topmost layer whose rendered pixels are under the
  pointer.
- On-canvas handles for move, scale, rotate, crop and text tilt, drawn over the viewer. A drag previews live and
  commits one revision on release. Snapping to canvas centre and edges, and to 15° rotation steps.
- Viewer guides: rule-of-thirds and 2–5 division grids, action-safe and title-safe areas, centre cross, and format
  masks (2.39:1, 1.85:1, 1:1, 9:16, 4:5) with editable presets for platform UI-safe zones. `ove look --overlay` renders
  them for agents. A new verify lint warns when text or captions fall outside title-safe.
- A full-screen viewer with floating transport controls, and playback rates 0.5×, 1×, 1.25×, 1.5× and 2×.
- Text fills: `color` (today), `footage` (the composite below shows through the letters) and `inverted` (difference
  blend). Perspective tilt for text with `rotationX` and `rotationY`. Static per-word styles on text and captions
  (`textSpans`: colour, weight, style, background, scale by word index).
- Lottie items, rendered frame by frame from the vector source, with `src`, `speed`, `loop`, `holdLast` and colour
  slot overrides. Unsupported Lottie features are reported by `check`.

Closes add-motion-graphics task 1.5 (Lottie import).

Out of scope: subject matte as a text fill source or text behind the subject (add-color-management-and-subject-mattes);
3D layers other than text tilt; mask drawing tools; keyframe editing on canvas (add-markers-keyframe-lanes-and-versions
owns keyframe lanes); pitch-preserving audio at rates other than 1× (audio plays muted above 2× and resampled at other
rates until add-audio-mixing-and-monitoring adds a time-stretcher).

References: docs/spec/project-schema-v1.md (`transform`, text style, caption items), docs/spec/timeline.d.ts,
docs/spec/projection-format.md, docs/spec/presets.md (text animations, fonts), docs/spec/cli.md (`look`),
docs/spec/desktop-shell.md (viewer, View menu).

## Capabilities

### New Capabilities
- `viewer-interaction`: layer geometry and hit testing, on-canvas handles, guides and safe areas with the safe-area
  lint, full-screen viewer and playback rates.
- `text-fills`: footage and inverted text fills, text perspective tilt, per-word span styles, and Lottie items.

### Modified Capabilities
- None. New fields are optional and old documents render as before.

## Impact

- `crates/ove-render/src/eval.rs`: `layer_geometry`. `crates/ove-render/src/canvas.rs`: projective draw.
  `crates/ove-render/src/text.rs`: fill modes and spans. New `crates/ove-graphics/src/lottie.rs`.
- `crates/ove-core/src/schema.rs`: `textFill`, `footageMatteColor`, `transform.rotationX`, `transform.rotationY`,
  `textSpans`, item type `lottie`. Projection and `timeline.d.ts` updated.
- `crates/ove-playback`: rate command. UI: `apps/desktop/ui/src/Viewer.tsx` overlay layer, full-screen mode.
- CLI and MCP: `ove geometry`, `ove look --overlay`, safe-area lint in `verify`.
- Depends on add-gpu-compositor-and-scopes for the difference blend on the GPU path (the CPU path gets it from
  fix-render-and-edit-correctness). Keymap entries for the new actions are registered through
  add-preferences-i18n-and-keymaps when it lands.
- New dependencies: ThorVG (MIT) through dotlottie-rs (MIT), behind a spike task.
