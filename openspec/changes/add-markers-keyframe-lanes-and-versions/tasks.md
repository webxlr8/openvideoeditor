# Tasks: add-markers-keyframe-lanes-and-versions

## 1. Markers in the document and projection

- [x] 1.1 Add the marker entity, ops and invariants to `ove-core`; verify unit tests for each op and each invariant
      refusal.
- [x] 1.2 Print and parse `<marker>` elements, update `timeline.d.ts` and `docs/spec/projection-format.md`; verify
      golden corpus cases (valid and invalid) and the 10,000-case round trip with marker generators.
- [x] 1.3 Generalise the item merge into a keyed-entity merge used for items and markers; verify the existing merge
      property suite still passes and a new property: concurrent edits to different markers never conflict.

## 2. Markers in edits and agents

- [x] 2.1 Map markers through planner time maps when requested (point removal, range shrink); verify property: a ripple
      never moves a marker before the removed range, and markers stay put when not following.
- [x] 2.2 Add markers to snap targets; verify a unit test in `ove-edit::snap`.
- [x] 2.3 `ove markers list|add|update|remove` with status and assignee filters, MCP marker tools returning items under
      each marker, refusal of agent `resolved`; verify CLI integration tests and an MCP contract test for the refusal.
- [x] 2.4 Assistant prompt rule for the review workflow; verify an agent test where a mock agent sets review after
      `look` and the resolve attempt is refused.
- [x] 2.5 Ruler lane, M key, popover editor and status filter; verify UI tests for add, edit, filter and delete.

## 3. Keyframes

- [x] 3.1 Per-keyframe easing with `hold`, crop-edge properties, schema and renderer evaluation; verify golden render
      frames for each easing and crop keyframes in preview and export.
- [x] 3.2 Merge `keyframeTracks` keyed by property; verify property tests: different properties never conflict, the
      same property conflicts by the existing policy.
- [x] 3.3 Keyframe lanes UI with add, drag, delete and a volume line on audio items; verify UI tests that each gesture
      is one revision and expansion creates none.
- [x] 3.4 `ove edit keyframes` and the MCP tool with range refusal; verify CLI integration tests and docs examples.

## 4. Versions and scoped undo

- [x] 4.1 `versions.json`, pinned snapshots with shared structure, and pruning that skips pinned revisions; verify a
      fault-injection test: a version survives pruning and a crash during save.
- [x] 4.2 Semantic diff classification (added, removed, moved, trimmed, changed) for items and markers; verify unit
      tests per class.
- [x] 4.3 `ove version save|list|print|diff|restore`; verify CLI integration tests and docs examples.
- [x] 4.4 Scoped undo by actor and by timeline in `ove-store`, replacing `undo_turn` internals; verify property: undo by
      actor never reverts another actor's revision, and a conflicting later edit refuses.
- [x] 4.5 History panel versions section with compare and restore, and actor filter; verify UI tests.

## 5. Integration checks

- [ ] 5.1 In the built app on macOS and Windows: marker review round trip with an MCP agent, keyframe lane edits, save
      and restore a version, agent-only undo; verify each behaves as specified and each gesture is one undo step.
- [x] 5.2 Size and speed: 20 versions of a 5,000-item project; verify bundle growth and `version diff` time, recorded
      in design.md Results.
