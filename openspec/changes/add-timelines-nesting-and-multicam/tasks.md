# Tasks: add-timelines-nesting-and-multicam

## 1. Timelines in the document

- [ ] 1.1 Add the timeline entity, track `timelineId`, invariants and the migration to `tl_main`; verify a migration
      test opens every existing corpus bundle and renders identical golden frames.
- [ ] 1.2 Per-timeline projection files and scoped apply in `ove-projection`, with `--timeline` on `ove source` and the
      MCP parameter; verify property: a scoped apply never changes another timeline (10,000 cases), and golden files
      for multi-file print.
- [ ] 1.3 Document the layout in `docs/spec/document-model.md` (Bundle layout) and `docs/spec/projection-format.md`;
      verify docs examples run in CLI integration tests.

## 2. Nesting

- [ ] 2.1 `nest` item type with `nest.timeline.exists` and `nest.acyclic` (depth ≤ 8) and refused timeline removal;
      verify invariant unit tests for each refusal.
- [ ] 2.2 Recursive evaluation in `ove-render` with a cache per child timeline and subtree revision; verify golden
      frames for nested content and a benchmark with 3 levels recorded in Results.
- [ ] 2.3 Nest and flatten planners in `ove-edit` that keep ids and warn on lost nest properties; verify property:
      flatten after nest is render-equivalent to the original.
- [ ] 2.4 Tab bar, double-click to open a nest, per-timeline view state; verify UI tests that opening and switching
      tabs creates no revision.

## 3. Multicam

- [ ] 3.1 Multicam timeline kind, member roles and labels, `multicam` program item and `multicam.angle.exists`
      invariant; verify unit tests and golden round trip.
- [ ] 3.2 Group creation using `sync_items` from `add-audio-intelligence`, with confidence report; verify a fixture with
      three synced cameras and one unrelated file.
- [ ] 3.3 Angle-switch planner with coverage refusal and joins only at planner seams; verify property: switching keeps
      program timing, and switch then switch back restores the original items.
- [ ] 3.4 Multicam viewer grid with click and number-key switching; `ove multicam create|switch|show` and MCP tools;
      verify UI tests, CLI integration tests and an MCP contract test.

## 4. Fork, port and save range as media

- [ ] 4.1 `ove version fork --as-timeline` and `ove version port` using the 3-way merge; verify properties: a port
      with no fork edits creates no revision; conflicts follow the chosen preference.
- [ ] 4.2 Timeline action "save range as media" with optional replace, using the range render from
      `extend-export-formats-and-queue`; verify an SSIM check of the rendered clip against an export of the same span,
      and that undo restores the replaced items.

## 5. Integration checks

- [ ] 5.1 In the built app on macOS and Windows: create a second timeline, nest and flatten, a three-camera multicam
      edit with angle switches, fork and port, save range as media; verify each behaves as specified and each action
      is one undo step.
- [ ] 5.2 Agent round trip: an MCP agent edits `timelines/<id>.tsx` while the user edits the main timeline for 10
      minutes; verify zero lost edits and no cross-timeline changes.
