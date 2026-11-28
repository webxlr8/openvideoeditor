# Proposal: extend-ai-generation-actions

## Why

Generation today makes new media from a prompt with a fixed list of 10 models and one optional image reference. The
model list is compiled into the app, so a new model needs a release. Editors also want to change footage they already
have: upscale a shot, reframe it to another aspect, re-sync lips to new audio, edit it with a prompt, add sound effects
or music that fit the picture, dub it, or clean up the voice. They want a generated shot to bridge two clips. Agents need
the same actions, with spending kept under the person's control.

## What Changes

- Model catalog with capabilities: modes (text, first frame, first + last frame, references, source video, source
  audio), limits (durations, resolutions, aspects, reference counts), price rules, action tags and typed settings. The
  app ships an embedded copy; an optional remote copy, signed with an ed25519 key compiled into the app, refreshes it at
  most once a day. Unsigned or tampered catalogs are ignored.
- References: several image, video and audio references per request, addressed in the prompt as `@Image1`, `@Video1`,
  `@Audio1`; uploads to the provider's storage with the person's own key; an upload cache by content hash with expiry.
- Preprocessing: send only the trimmed range of a clip, or a composited timeline span, downscaled and transcoded to what
  the model accepts, using the shared renderer.
- Spend control: a local spend ledger per project, estimates before every paid call, budgets per project and per agent
  session, and a confirmation by the person when a budget would be exceeded.
- AI actions on a clip: upscale, reframe / extend, lip sync, prompt edit, video-to-sound-effects, video-to-music,
  dubbing, voice cleanup, voice changer, voice clone (with consent), rerun, and draft-to-enhance. Visual results land as
  a "take" placeholder on the lane above the clip; "replace when done" swaps the clip's media and keeps its edits; audio
  results land on an audio lane at the same span.
- AI transitions: fill a gap, or bridge a cut, with a clip generated from the last frame before and the first frame
  after, fitted by speed or by trim.
- Surfaces: `ove generate action <kind> --item <id>`, `ove generate transition`, `ove generate models --refresh`; MCP
  `ai_clip_action`, `generate_transition`, catalog-driven `list_models`; a timeline context submenu, an Inspector "AI"
  section and an updated Generate panel built from catalog capabilities.

Out of scope: the base generation MCP tools and the quote / confirm contract (`unify-agent-tool-layer`); fixes to job
reaping and the generation MCP task mismatch (`fix-render-and-edit-correctness`); stock media search; local audio
separation; any hosted proxy, accounts or credits; local generative models.

References: docs/spec/cli.md (`generate`), docs/spec/mcp-tools.md (`generate_*`, `list_voices`, `job_status`,
`cancel_job`), docs/spec/presets.md (generation models), docs/spec/document-model.md (pending placeholders, media
registry).

## Capabilities

### New Capabilities
- `ai-clip-actions`: the signed model catalog, multi-reference requests, preprocessing, spend control, AI actions on
  existing clips with takes and replace, and AI transitions, with their CLI, MCP and UI surfaces.

### Modified Capabilities
- None (extends the generation behaviour of `add-ai-generation` without changing its requirements).

## Impact

- `crates/ove-generate`: new `catalog/{schema,embedded,remote}.rs`, `refs.rs`, `prep.rs`, `spend.rs`, `actions.rs`,
  `providers/fal_storage.rs`; `place.rs` gains take and replace placement; `Request.image` becomes `refs`.
- `crates/ove-cli` (`generate action|transition|models --refresh`), MCP tool handlers in the registry of
  `unify-agent-tool-layer`.
- `apps/desktop/ui`: Generate panel rebuilt from capabilities, mention picker, timeline AI submenu, Inspector AI section,
  take lane rendering.
- Release CI: catalog signing step (shared with `add-distribution`).
- Depends on: `extend-export-formats-and-queue` (range render), `unify-agent-tool-layer` (tool registry, quote and
  confirm), `fix-render-and-edit-correctness` (job resume by remote id).
