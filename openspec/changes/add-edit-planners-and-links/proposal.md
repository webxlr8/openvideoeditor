# Proposal: add-edit-planners-and-links

## Why

Today the timeline has only single-item commands: move, trim and split. There is no ripple, no insert or overwrite,
no slip, slide or roll, and no link between a clip's picture and its separate sound. Every writer (GUI, assistant, CLI,
MCP) must compute downstream start times by hand, which is slow for people and error-prone for agents. Transcript
editing, silence removal and multicam all need the same edit primitives. This change adds them once, as pure planners
that every writer shares, and makes concurrent ripples merge correctly.

## What Changes

- A frame rate per timeline (`fps` on `<timeline>`). Edits snap to the frame grid; off-grid times become a lint, not
  an error.
- Edit planners shared by all writers: split, trim, ripple trim, ripple delete (items or ranges), insert, overwrite,
  lift, extract, close gap, slip, slide, roll, and snap. Each planner returns a plan (ops, summary, time map) or a
  structured refusal (code, culprit item ids, message). One plan is applied as one revision.
- Retiming that respects playback speed and reverse, and moves keyframes, caption segments, fades and transitions
  with the cut. This replaces the per-command logic fixed in `fix-render-and-edit-correctness`.
- Sync lock per track (`syncLock`, default on). A ripple shifts the target track and every sync-locked track.
- Link groups: an item field `link`. Linked items move, trim, slip and delete together by default; an override edits
  one side only (J and L cuts). Lints for single-member groups and for drift between members.
- A time map recorded on each ripple revision. Apply passes file-side times through the time maps of later
  revisions, so an agent edit made before a concurrent ripple lands in the right place.
- Range selection (in and out points), tool modes (select, blade, trim), new rebindable keys, and a two-up viewer that
  shows the outgoing and incoming frames during slip, slide and roll.
- One `edit_timeline` operation on the CLI (`ove edit …`), MCP and the assistant, with a preview mode that returns the
  plan without applying it.

Out of scope: markers and keyframe lanes (`add-markers-keyframe-lanes-and-versions`), multiple and nested timelines
and multicam (`add-timelines-nesting-and-multicam`), beat detection (`add-audio-intelligence`; this change only
accepts beat times as snap targets when present), transcript-driven cuts (`add-transcript-editing` calls these
planners), timeline waveforms (`add-audio-mixing-and-monitoring`).

References: docs/spec/document-model.md (Items, Projection rules, Apply / merge, Verify), docs/spec/projection-format.md
(timeline and track attributes), docs/spec/cli.md, docs/spec/mcp-tools.md (`apply_timeline`), docs/spec/presets.md
(Keyboard shortcuts).

## Capabilities

### New Capabilities
- `edit-planners`: frame grid, the planners and their refusals, retiming rules, sync lock, time-map rebase during
  apply, range selection, tool modes, the two-up viewer and the `edit_timeline` surfaces.
- `clip-links`: link groups, linked editing and its override, link lints and link creation on import.

### Modified Capabilities
- None. The timeline projection gains attributes (`fps`, `syncLock`, `link`) under its existing unknown-field and
  merge rules; the merge gains a rebase step described as part of `edit-planners`.

## Impact

- New crate `ove-edit` (pure; depends on `ove-core` only). Changes in `ove-core` (schema fields, revision time map),
  `ove-projection` (print and parse new attributes, rebase in merge), `apps/desktop/src-tauri` (`edits.rs`,
  `projects.rs` move onto planners), `apps/desktop/ui` (`Timeline.tsx`, `keys.ts`, viewer two-up), `ove-cli`
  (`ove edit`, MCP `edit_timeline`), `ove-agent` (tool).
- Depends on `fix-render-and-edit-correctness` (retiming fix moves here). `unify-agent-tool-layer` registers
  `edit_timeline` in its registry if it lands first; otherwise this change adds the tool to the current MCP table and
  assistant tools.
- Used by `add-transcript-editing`, `add-markers-keyframe-lanes-and-versions`, `add-timelines-nesting-and-multicam`
  and `add-audio-intelligence` (beat snapping).
