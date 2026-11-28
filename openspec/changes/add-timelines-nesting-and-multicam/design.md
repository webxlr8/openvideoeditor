# Design: add-timelines-nesting-and-multicam

## Context

See proposal.md (Why). Builds on `project-document`, `timeline-projection`, `edit-planners`, `timeline-markers`,
`project-versions`. Requirements: specs/multiple-timelines, specs/multicam.

Current state: the document has one implicit timeline; `<timeline>` contains only `<track>` elements; `aspect` is a
project field. Merge is a keyed 3-way merge per item and field (generalised to keyed entities by
`add-markers-keyframe-lanes-and-versions`).

## Goals / Non-Goals

**Goals:**
- Several timelines with no risk that editing one changes another by accident.
- Nesting and flattening that keep item identity.
- Multicam angle switches that are small, readable diffs.

**Non-Goals:**
- Separate revision logs per timeline. One project, one log.
- A nested timeline with its own media library.

## Decisions

### D1. Timelines as entities, tracks point to them
`project.timelines` is a keyed map `{id, name, aspect, fps, kind}`. Each track has `timelineId`. Items belong to a
timeline through their track. Migration wraps existing tracks into `tl_main` and moves the project `aspect` onto it.
Alternatives: nested arrays of tracks inside timelines (moves track identity under a parent; merge gets an extra
level); one document per timeline (cross-timeline nesting and one history become hard).

### D2. One file per timeline, scoped apply
`timeline.tsx` prints `tl_main`, so every existing caller keeps working. Other timelines print to
`timelines/<id>.tsx` with root `<timeline id name aspect fps kind>`. Applying a file treats only its own timeline's
tracks and items as present or absent. The header `rev` stays global, so there is one merge base.
Alternative: all timelines in one file (large files, and an agent edit could delete another timeline by omission).

### D3. Nest items
New item type `nest`: `<nest id="n1" timeline="tl_intro" start={10} end={18} in={2}/>`. Invariants:
`nest.timeline.exists` and `nest.acyclic` (depth-first check, depth ≤ 8). Deleting a timeline that a nest uses is
refused and lists the nest items. Rendering evaluates the child timeline with a time offset and fits its canvas into
the parent; results are cached per child timeline and the revision of its subtree.
Alternative: flatten at render time with prefixed ids (identity differs between preview and document).

### D4. Nest and flatten keep ids
`nest(selection)` creates a timeline and tracks, moves the selected items by changing their `trackId`, shifts their
times to start at 0, and inserts one `nest` item. Ids do not change. `flatten(nest)` moves items back with remapped
times and trims, and reports properties on the nest item (effects, grade, transform) that cannot be pushed down.
Alternative: copy with new ids (breaks markers, history and agent references).

### D5. Multicam as a timeline kind
A multicam group is a timeline with `kind="multicam"`. Each track is one member with `role` angle, mic or both, and a
label. Sync offsets are the members' item positions, set by `sync_items` from `add-audio-intelligence`. The program is
an item `<multicam id timeline angle audio={[…]} start end in/>`. Switching angle over a range is a split by the
planner plus an `UpdateItem` of `angle`. Joining adjacent program items is done only at seams the planner created and
only when both sides are equal in every field.
Alternatives: switch angles by rewriting media references and trims (large diffs, fragile re-joins); one item per
angle per range on several tracks (cross-track atomicity rules).

### D6. Coverage
An angle switch over a range where the angle has no media is refused with `multicam.noCoverage` and the gap times.

### D7. Fork and port
`ove version fork <v> --as-timeline` creates a timeline from the version's main timeline with `forkedFrom {timeline,
rev, idMap}` and pins the base revision. `ove version port <fork>` prints the fork, maps ids back through `idMap`, and
runs the 3-way merge with base = the pinned revision onto the source timeline. Conflicts follow the existing policy.

### D8. Save range as media
The export side (render a range to `media/rendered/`, register it) belongs to `extend-export-formats-and-queue`. This
change adds the timeline action and the optional replace: remove the covered items in the range on the chosen tracks
and insert the rendered clip, in one revision.

### D9. Surfaces
CLI: `ove timeline new|rename|rm|list|nest|flatten`, `ove source print|apply --timeline <id>`, `ove multicam
create|switch|show`. MCP: `timeline` parameter on print and apply; multicam tools. GUI: a tab bar, double-click to
open a nest, angle switching from a multicam viewer grid and number keys.

## Risks / Trade-offs

- [An agent applies a stale per-timeline file] → each file's header `rev` is checked as today. Detected by a scoped
  apply property test.
- [Recursive render cost] → cache per child timeline and subtree revision; depth limit 8. Detected by a render
  benchmark with 3 levels of nesting.
- [Track id rename leaves a dangling angle] → track ids are stable; invariant `multicam.angle.exists` refuses it.
  Detected by an invariant unit test.
- [Migration of existing bundles] → migration is idempotent and runs on open; the old projection path is unchanged.
  Detected by opening the existing corpus bundles in a migration test.

## Migration Plan

On open, documents without `timelines` gain `tl_main`, every track gets `timelineId = tl_main`, and the project
`aspect` is copied onto `tl_main`. The project `aspect` stays as a read alias for one release.

## Open Questions

- Owner decisions still open across the plan, with the defaults this change assumes: waves proceed in master-plan
  order; models and runtimes download on first use with explicit consent above 50 MB; DirectML off by default pending
  a licence ruling; SVT-AV1 excluded pending a ruling; remote model catalog is an embedded copy plus an optional signed
  remote URL (host to be decided); community skill catalog URL is configurable and 3 original starter skills ship.
  None of these block this change.
- Whether nests can have a different frame rate from the parent (assumed yes, with frame-accurate resampling of time).
