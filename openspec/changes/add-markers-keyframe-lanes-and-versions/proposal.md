# Proposal: add-markers-keyframe-lanes-and-versions

## Why

Editors and agents need a shared place to leave notes on the timeline and to hand work back and forth. Keyframes exist
in the document and the renderer, but people cannot see or edit them on the timeline, and concurrent keyframe edits on
different properties conflict. History keeps every revision but prunes old ones, so there is no durable "version 2
sent to the client" to return to or compare with, and an agent's undo can revert another writer's edit. This change
adds markers with a review workflow, keyframe lanes, named versions and undo scoped to one actor or timeline.

## What Changes

- Markers as their own document entities: point or range, name, colour, note, status (open, review, resolved) and an
  optional assignee. They print as `<marker>` elements in the projection, merge per marker, ripple with edits through
  the planners' time map, and act as snap targets. Agent rule: an agent may move a marker to review after it checks the
  edit, and never resolves it.
- Marker commands on the CLI and MCP (list, add, update, remove, filter by status and assignee), a ruler lane, the M
  key, a popover editor and a status filter.
- Keyframes: optional easing per keyframe (overrides the track easing), a new `hold` easing, and the properties
  `volume` (from `add-audio-mixing-and-monitoring`) and crop edges. Keyframe tracks merge per property instead of as one
  array value.
- Keyframe lanes under each track (view state only): diamonds to drag, add and delete, and a volume line on audio.
  `ove edit keyframes` and an MCP tool.
- Named versions: pin a revision with a name and a snapshot that pruning never removes, list, print, diff
  (added, removed, moved, trimmed, changed) and restore as one new revision.
- Undo scoped by actor (one agent session, one assistant turn, or the user) and by timeline, using inverse ops that
  refuse on conflict.

Out of scope: forking a version into a new timeline and porting it back (`add-timelines-nesting-and-multicam`),
threaded comments on markers, marker import or export to other editors (`improve-interchange-and-packaging`), the
volume mixer evaluation (`add-audio-mixing-and-monitoring` owns it).

References: docs/spec/document-model.md (project.json, Items, Projection rules, Apply / merge, Verify),
docs/spec/projection-format.md, docs/spec/cli.md (`history`, `undo`), docs/spec/mcp-tools.md (`undo_timeline`),
docs/spec/presets.md (Keyboard shortcuts).

## Capabilities

### New Capabilities
- `timeline-markers`: marker entities, projection, merge, ripple, snapping, review workflow and surfaces.
- `keyframe-lanes`: per-keyframe easing and hold, new properties, merge per property, lanes UI and keyframe commands.
- `project-versions`: named versions, diff and restore, and undo scoped by actor and timeline.

### Modified Capabilities
- None.

## Impact

- `ove-core` (marker entity and ops, keyframe schema, revision actor read), `ove-projection` (marker print and parse,
  keyed-entity merge, keyframe merge by property), `ove-store` (versions file, pinned snapshots, scoped inverse undo),
  `ove-edit` (markers in time maps), `apps/desktop` (ruler lane, popover, lanes, History panel versions section),
  `ove-cli` (`ove markers`, `ove version`, `ove edit keyframes`, MCP tools), `ove-agent` (prompt rule).
- Depends on `add-edit-planners-and-links` (time map, frame grid, snap engine) and on `unify-agent-tool-layer` (actor
  on revisions; if it has not landed, this change adds the actor field itself under the same name).
- `add-audio-mixing-and-monitoring` adds `volume` to the keyframe properties; this change adds the lane UI for it.
