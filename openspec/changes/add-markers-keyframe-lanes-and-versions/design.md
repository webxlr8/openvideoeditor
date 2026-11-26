# Design: add-markers-keyframe-lanes-and-versions

## Context

See proposal.md (Why). Builds on `project-document`, `timeline-projection` and `edit-planners`. Requirements:
specs/timeline-markers, specs/keyframe-lanes, specs/project-versions.

Current state: no markers. `keyframeTracks` is an array of `{property, keyframes[{time, value}], easing}` with easing
per track, evaluated in `ove-render/src/eval.rs`. Revisions record a `source` but no actor. History prunes after
30 days or 2,000 changes (`ove-store/src/project.rs`). Per-turn assistant undo exists (`assistant.rs::undo_turn`).

## Goals / Non-Goals

**Goals:**
- Markers that users and agents can edit at the same time without conflicts on different markers.
- Keyframe edits on different properties of one item that merge cleanly.
- Versions that survive pruning and can be compared.

**Non-Goals:**
- Global ripple-marker preference. Each ripple request states whether markers follow.
- Comment threads, mentions or notifications on markers.

## Decisions

### D1. Markers as keyed entities
`project.markers` is an ordered map from id to marker, with a typed head (`id`, `at`, `dur`, `status`) and a field map
(name, color, note, assignee, unknown fields kept). Ops: `AddMarker`, `UpdateMarker`, `RemoveMarkers`.
Invariants: `at ≥ 0`, `dur ≥ 0`, `status` in the enum.
Alternatives: markers as items on a special track (they would merge and render like media, and confuse lints); a
marker array field on the project (arrays merge as whole values, so two writers would conflict).

### D2. Projection
Markers print as children of `<timeline>`, before tracks, ordered by time then id:
`<marker id="m1" at={12.4} dur={3} name="Tighten" color="#e5484d" status="review" assignee="agent" note="…"/>`.
Notes longer than 200 characters print as a braced string on their own line.

### D3. One keyed-entity merge
Generalise the item merge in `merge.rs` into a keyed-entity merge (base, file, current per id and per field) and use it
for items and markers. Later timelines reuse it.
Alternative: a second merge implementation for markers (two code paths to prove).

### D4. Markers in ripples
Planners map markers through the same time map as items when the request sets `markers: true` (GUI default on).
A point marker inside a removed range is removed; a range marker that overlaps a removed range shrinks.
Alternative: an app-wide setting (agent results would depend on a user preference).

### D5. Review workflow for agents
Statuses: open → review → resolved. The assistant prompt and the MCP tool description state: move to review only after
checking the edit with `look`; never set resolved. The tool layer refuses `resolved` from an agent actor with code
`marker.resolveByAgent`.
Alternative: prompt rule only (agents may still resolve).

### D6. Keyframes
Each keyframe may carry `easing` (linear, ease-in, ease-out, ease-in-out, hold) that overrides the track easing.
New properties: `volume` (dB, added by `add-audio-mixing-and-monitoring`) and `crop.left|right|top|bottom`.
Merge treats `keyframeTracks` as a map keyed by `property`, so edits to different properties merge; one property
remains one value.
Alternatives: merge per keyframe (keyframes have no ids; matching by time is fragile); keep whole-array merge (common
user-and-agent conflicts).

### D7. Lanes are view state
Lane expansion per track lives in UI state, not in the document. Diamonds drag in time and value; Alt-click adds;
Delete removes. One gesture is one revision.

### D8. Versions
`.revs/versions.json` holds `{name, rev, createdAt, actor, note}`. Saving a version writes a full snapshot of that
revision so pruning never removes it. `diff` uses `ove-core` diff and classifies changes as added, removed, moved,
trimmed or changed per item and marker. `restore` uses the existing restore path, so it is one undoable revision.
Alternatives: copy the timeline with new ids (loses identity, so no diff); git under the hood (a dependency and a
second history).

### D9. Scoped undo
Read `RevisionRecord.actor` (`kind`, `session`, `turn`, `client`). Scoped undo collects the latest revisions that match
the scope, applies their inverse ops to the current document as one revision, and refuses with the conflicting item
ids if a later revision from another actor changed the same fields. This generalises `undo_turn` and moves it into
`ove-store`.

## Risks / Trade-offs

- [Pinned snapshots grow the bundle] → share unchanged structure with the previous snapshot; show size per version.
  Detected by a size test on 20 versions of a 5,000-item project.
- [Old bundles have no actor] → missing actor reads as "unknown" and is never matched by an agent scope. Detected by a
  migration test with old bundles.
- [Long notes bloat the projection] → wrapped braced strings; note length limit 4,000 characters with a refusal.
  Detected by corpus cases.
- [Keyframe merge change alters existing conflict behaviour] → covered by the merge property suite with keyframe
  generators. Detected by the 10,000-case property run.

## Migration Plan

All new fields are optional. `keyframeTracks` keeps its shape. `versions.json` is created on first save.

## Open Questions

- Owner decisions still open across the plan, with the defaults this change assumes: waves proceed in master-plan
  order; models and runtimes download on first use with explicit consent above 50 MB; DirectML off by default pending
  a licence ruling; SVT-AV1 excluded pending a ruling; remote model catalog is an embedded copy plus an optional signed
  remote URL (host to be decided); community skill catalog URL is configurable and 3 original starter skills ship.
  None of these block this change.
- Marker colour palette: 8 named colours plus any hex value (assumed).
