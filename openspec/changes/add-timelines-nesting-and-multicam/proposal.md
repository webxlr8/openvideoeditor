# Proposal: add-timelines-nesting-and-multicam

## Why

A project has one implicit timeline. Real work needs several: a 16:9 master and a 9:16 cut-down, an intro built once
and reused, a podcast shot on three cameras. Today there is no way to keep alternative cuts side by side, to nest a
group of items as one, or to switch between synced camera angles. Agents also need to edit one timeline without any
risk to the others. This change adds multiple timelines, nesting that keeps item identity, multicam groups, forking a
version into a timeline and merging it back, and saving a range as media.

## What Changes

- Multiple timelines per project, each with a name, aspect, frame rate and kind (edit or multicam). Existing projects
  migrate to one timeline `tl_main`.
- One projection file per timeline: `timeline.tsx` stays the main timeline for every existing caller; other timelines
  print to `timelines/<id>.tsx`. Applying a file only adds, changes or removes content inside its own timeline. The
  revision number stays global.
- A `nest` item that places one timeline inside another, with checks that the referenced timeline exists and that
  nesting has no cycle (depth at most 8). Nested content renders in preview, `look` and export.
- Nest and flatten planners. Nesting moves the selected items into a new timeline and keeps their ids, so markers,
  history and agent references stay valid. Flatten puts remapped copies back and reports looks on the nest that
  cannot be kept.
- Multicam groups as timelines of kind multicam: each track is one member with a role (angle, mic or both); sync is
  the members' positions, set by audio sync from `add-audio-intelligence`. A `multicam` program item selects the angle
  and audio members. Switching angle over a range is a split plus a one-field change.
- Fork a named version into a new timeline, and port the fork's changes back with the existing 3-way merge.
- Save a timeline range as media, with an optional replace of the range by the rendered clip, in one revision.
- A tab bar for open timelines, double-click a nest to open it, and multicam angle switching from the viewer and keys.

Out of scope: multicam layout presets (picture-in-picture grids built from angles), an ML beat model, nesting across
projects, collaborative editing of one timeline by several machines.

References: docs/spec/document-model.md (Bundle layout, project.json, Items, Projection rules, Apply / merge),
docs/spec/projection-format.md, docs/spec/cli.md (`source`), docs/spec/mcp-tools.md (`print_timeline`,
`apply_timeline`), docs/spec/presets.md (Keyboard shortcuts).

## Capabilities

### New Capabilities
- `multiple-timelines`: timeline entities and migration, per-timeline projection files and scoped apply, nest items
  and their checks and rendering, nest and flatten, version fork and port, save range as media, and the tab UI.
- `multicam`: multicam timelines, member roles, the program item, angle and audio switching, and the surfaces.

### Modified Capabilities
- None.

## Impact

- `ove-core` (timeline entity, track `timelineId`, nest and multicam item types, invariants, migration),
  `ove-projection` (multi-file print, scoped apply, timeline root attributes), `ove-render` (recursive evaluation,
  cache by timeline subtree), `ove-edit` (nest, flatten, angle-switch planners), `ove-store` (fork and port),
  `apps/desktop` (tabs, nest open, multicam viewer), `ove-cli` (`ove timeline …`, `--timeline` on source commands,
  MCP parameter), `ove-agent` (timeline parameter on tools).
- Depends on `add-edit-planners-and-links`, `add-markers-keyframe-lanes-and-versions` (versions for fork),
  `add-audio-intelligence` (audio sync for multicam members) and `extend-export-formats-and-queue` (range render for
  save-as-media; that change owns the export side of save range as media, this change owns the replace step and the
  timeline UI).
