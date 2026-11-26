# Tasks: add-edit-planners-and-links

## 1. Schema and projection

- [x] 1.1 Add `fps` (timeline), `syncLock` (track) and `link` (item) to the schema, printer, parser and `timeline.d.ts`
      with defaults omitted; verify golden round trip of the existing corpus is unchanged and new corpus cases cover
      each field.
- [x] 1.2 Replace the constant 30 fps in `src-tauri/src/timeline.rs` and `ui/src/edit.ts` with the timeline value;
      verify a 25 fps project steps one frame per 40 ms in a UI test.
- [x] 1.3 Register lints `time.offFrame`, `link.single`, `link.offset`; verify corpus cases report each lint.
- [x] 1.4 Document the new attributes in `docs/spec/projection-format.md` and `docs/spec/document-model.md`; verify the
      docs examples parse with `ove source check`.

## 2. Planner crate

- [x] 2.1 Create `ove-edit` with the plan and refusal types and the retiming helper (speed, reverse, keyframes, caption
      segments, fades, transitions); verify property test: evaluating any keyframed property at any timeline time is
      unchanged by a split or trim elsewhere.
- [x] 2.2 Implement split, trim, ripple trim, ripple delete (items and ranges), insert, overwrite, lift, extract and
      close gap with sync lock and link expansion; verify properties: length change equals removed length; gaps
      between items on sync-locked tracks are preserved; apply then inverse returns the original document.
- [x] 2.3 Implement slip, slide and roll with source-limit clamping; verify properties: slip keeps start and end; roll
      keeps the sum of the two durations; slide keeps duration and source range.
- [x] 2.4 Implement snap target collection (playhead, item edges, range points, optional beat times) with sticky
      threshold; verify unit tests ported from `ui/src/snap.test.ts`.
- [x] 2.5 Post-plan overlap check and refusal codes; verify a property test over random documents finds no new overlap
      unless a crossfade was requested.
- [x] 2.6 Move `split_item`, `trim_item` and `move_item` in `src-tauri` onto the planners; verify the existing desktop
      tests pass and the old helpers are deleted.

## 3. Rebase in merge

- [x] 3.1 Record a time map on each ripple revision in `ove-core`; verify old revision records still decode (fuzz the
      decoder with old and new records).
- [x] 3.2 Map file-side times through later time maps in `ove-projection` merge; verify property: GUI ripple then agent
      edit equals the sequential result (10,000 cases), and added items keep their times.
- [x] 3.3 Report a ripple's touched set as one conflict group; verify with a forced-conflict property test.

## 4. Links

- [ ] 4.1 Create linked video and audio items on import and drop; verify a desktop test imports a camera fixture and
      finds one link value on two items.
- [ ] 4.2 Linked editing in all planners with the `linked: false` override; verify property: move and ripple keep
      `start − in` equal across a group unless the override is set.
- [ ] 4.3 Timeline badges for links and drift chips; verify a UI test shows "+3f" after a file edit drifts a pair.

## 5. Surfaces

- [ ] 5.1 Tauri `edit_plan` and `edit_preview` commands with ghost rendering of previews; verify a UI test shows the
      ghost and that preview creates no revision.
- [ ] 5.2 Tool modes (V, B, T), range in/out, new default keys in the keymap, all rebindable; verify UI tests for each
      key and mode.
- [ ] 5.3 Two-up viewer layout fed by the frame-grab path, latest request wins; verify a drag shows both frames within
      100 ms on the reference machines (record numbers in Results).
- [ ] 5.4 `ove edit <verb>` with `--json` and `--dry-run`, MCP and assistant `edit_timeline`; verify CLI integration
      tests for success, refusal and stale-rev rebase paths, and an MCP contract test.
- [ ] 5.5 Document `ove edit` in `docs/cli/` and the tool in `docs/spec/mcp-tools.md`; verify docs examples run in the
      CLI integration tests.

## 6. Integration checks

- [ ] 6.1 Concurrency soak 30 minutes: GUI ripples racing agent print-edit-apply rounds; verify zero lost edits and zero
      invariant failures against an oracle model.
- [ ] 6.2 In the built app on macOS and Windows: ripple delete, insert, overwrite, slip, slide, roll, J and L cuts,
      undo of each; verify each is one undo step and the two-up view appears.
