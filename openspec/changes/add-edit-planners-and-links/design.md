# Design: add-edit-planners-and-links

## Context

See proposal.md (Why). Builds on `project-document` (ops, invariants, revisions) and `timeline-projection` (print,
parse, 3-way item merge). Requirements: specs/edit-planners, specs/clip-links.

Facts about the current code that shape the design:
- Times are integer microseconds. The frame rate is a constant 30 in `src-tauri/src/timeline.rs` and `ui/src/edit.ts`.
- Items on one track may overlap, and an overlap renders as a crossfade. So "no overlap" cannot be an invariant.
- A video item carries its own sound. Separate audio items exist for imported audio and for detached sound.
- Merge compares fields per item, one key deeper for objects; arrays compare as whole values.

## Goals / Non-Goals

**Goals:**
- One implementation of every edit verb, used by GUI, assistant, CLI and MCP.
- Planner behaviour stated as properties and proven by generated tests.
- Concurrent edits stay correct when one of them is a ripple.

**Non-Goals:**
- New projection syntax for verbs. The file stays declarative; ripple is a verb, not data.
- Frame-integer storage. Times stay in microseconds.

## Decisions

### D1. A pure planner crate
New crate `ove-edit`. Each planner is a function `(document, request) -> Plan | Refusal`. A plan holds the ops, a human
summary, the touched item ids and a time map. No I/O. The caller applies a plan as one batch, so invariants and
atomicity come from `project-document`.
Alternatives: planners inside `ove-core` (mixes edit policy with the document contract; harder to version); planners
in the UI (agents and CLI could not use them).

### D2. One retiming helper
`retime(item, head_cut, tail_cut)` computes the new `start`, `end`, `in`, `out` for any cut, using `playbackSpeed`
and reverse. It moves item-relative data: keyframe times, caption segment times, fades, and transition fields (a
right half never keeps the left half's `transitionIn`). Every planner that cuts or trims uses it.
Alternative: per-planner math (this is how the split and trim bugs happened).

### D3. Frame grid per timeline
`fps` is a timeline field (`<timeline fps={25}>`), default 30 for old documents. Planners snap requested times to the
grid. Times off the grid are allowed (imports, agents) and reported by the lint `time.offFrame`.
Alternative: store frames as integers — rejected; changing the frame rate would need a rescale of every item.

### D4. Sync lock and link expansion
`syncLock` is a track field, default true. A ripple shifts the target track, then every sync-locked track, then any
track that holds a link partner of a shifted item, until no new track is added. Before it emits ops, the planner checks
each follower track: an item would cross time 0, or a shift would push an item into a locked track. If so, it refuses
with code `ripple.blocked` and the culprit ids.
Alternative: ripple only the target track (desyncs sound from picture).

### D5. Link groups as a plain field
`link` is a string on items. Items with the same value are one group. There is no invariant: a single-member group
is a lint (`link.single`); a group whose members' `start − in` differ is a lint (`link.offset`, reports frames).
Planners expand to the group by default; the request flag `linked: false` (Alt in the GUI) edits one item.
Import and drop of a file with picture and sound create a video item and an audio item with one new link value.
Alternatives: a separate link table in the project (one more entity to merge, and agents must read two places);
an invariant (breaks when an agent deletes one side through the file; a lint is enough).

### D6. Time maps and rebase in merge
Each ripple revision records a time map: per track, a list of `(after, delta)` steps. During apply, before the 3-way
decision, file-side `startTime`/`endTime` of items that also exist in the base pass through the time maps of every
revision between the file's base `rev` and the current revision. Items that the file did not change are not touched.
Alternatives: refuse stale applies (breaks the agent workflow); re-run the file's edit as a verb (the file has no
verbs).

### D7. Refusals are data
A refusal has `code`, `culprits` (item ids) and `message`. The GUI shows it as a toast and outlines the culprits; the
CLI exits 1 with JSON; MCP returns it as a tool error.

### D8. Surfaces
- Tauri: `edit_plan(kind, request)` and `edit_preview(kind, request)` (returns the plan, used for ghost rendering).
- CLI: `ove edit <verb> <project> …` with `--json` and `--dry-run`.
- MCP and assistant: `edit_timeline { project, rev, op, … , apply }`.

### D9. Tool modes and two-up viewer
UI tool modes: select (V), blade (B), trim (T). In trim mode, dragging a clip body slips, Alt-drag slides, dragging an
edit point rolls. New default keys: Q/W ripple trim to playhead, Shift+Delete ripple delete, I/O range, X mark clip,
`,`/`.` insert/overwrite from source. All rebindable. The two-up view is a viewer layout that shows two decoded frames
side by side, fed from the frame-grab path, latest request wins.

## Risks / Trade-offs

- [Rebase moves an item the agent placed on purpose] → only items present in the base are mapped; added items keep
  their times. Detected by the property "GUI ripple then agent edit equals sequential edit".
- [Partial ripple after a conflict] → a ripple's touched set is reported as one conflict group. Detected by a merge
  property test with forced conflicts.
- [Planner creates an overlap the user did not ask for] → post-plan check refuses unless the request is a crossfade.
  Detected by a property test over random documents.
- [Two-up decode latency on Windows] → cancel stale requests, prefer proxies when present. Detected by the 100 ms
  measurement in tasks.
- [Old documents without `fps`] → default 30; no migration. Detected by golden round trip of existing corpus.

## Migration Plan

New fields are optional with defaults. No document migration. Old projections parse unchanged.

## Open Questions

- Owner decisions still open across the plan, with the defaults this change assumes: waves proceed in master-plan
  order; models and runtimes download on first use with explicit consent above 50 MB; DirectML off by default pending
  a licence ruling; SVT-AV1 excluded pending a ruling; remote model catalog is an embedded copy plus an optional signed
  remote URL (host to be decided); community skill catalog URL is configurable and 3 original starter skills ship.
  None of these block this change.
- Default for `syncLock` on newly created audio tracks: true (assumed).

## Results

Measured on the Windows 11 x64 development machine (not a reference machine), 2026-11-26.

**Group 1 (schema and projection).**
- Storage: `fps` is a project field (the document has one timeline), changed with `updateProject`; it prints as
  `<timeline fps={…}>` and merges as a single value like `aspect`. An absent `fps` and `fps={30}` compare equal, so
  printing the default never creates a change. `syncLock` is a track field (default `true`, printed only when
  `false`); `link` is an item string field printed after `pendingDescription`.
- Golden corpus: the four existing valid documents print byte for byte as before; new case `edit-fields` covers all
  three fields; new invalid case `fps-range` reports `value.range`. One existing invalid case
  (`attr-unknown-on-timeline`) used `fps` as its example of an unknown attribute and now uses `framerate`.
- Lints `time.offFrame`, `link.single` and `link.offset` are registered in verify; `link.offset` measures drift as
  the difference in `start − in ÷ speed` and reports frames to one decimal. An apply that leaves one member alone in
  its group lists it in the plan (`orphanedLinks`).
- The desktop timeline preview, the engine's frame interval and the UI's `snapToFrame` use the timeline's `fps`. UI
  test: on a 25 fps project, 25 frame steps read `00:00:01:00` (40 ms per step).
- Tests: `cargo test -p ove-core -p ove-projection` all pass (new: `edit_fields` 8 tests, corpus 2 tests); UI
  `pnpm test` 139 passed (21 files).

**Group 2 (planner crate).**
- `ove-edit` (depends on `ove-core` only) plans split, trim, ripple trim, ripple delete (items or ranges), delete,
  insert, overwrite, lift, extract, close gap, slip, slide, roll and move. A planner edits a copy of the document and
  the ops are the diff of the copy against the original, so one plan is one batch; `Plan` also carries the summary,
  touched ids, time map, the ids an edit was clamped on (`limited`) and the resulting document (for previews).
- Ripples are built from two primitives over a track set: open a gap at a time (items crossing it are cut) and
  remove a span (items inside are removed, items crossing an end are trimmed, an item covering the span is cut in
  two). Sync-locked tracks take the same edit, so a music bed under a ripple-deleted clip loses the same time and
  stays in sync. Close gap refuses (`ripple.blocked`) when a sync-locked track has content in the gap, because
  closing a gap must not remove anything.
- Right halves of a split link group share a new link value, so the two halves edit as two pairs.
- Known limit: a cut inside an eased keyframe segment keeps the values at the keyframes and the cut, but the curve
  between them is re-eased (easing is per track, not per segment). The keyframe property test uses linear tracks.
- `ove_core::retime` keeps only `ease` (the renderer uses it); split and trim retiming moved to `ove_edit::retime`
  with their tests. The desktop's `split_item`, `trim_item` and `move_item` call the planners; the old `trimmed`
  helper and its tests moved into `ove-edit/tests/trim.rs` unchanged in substance.
- Tests: `cargo test -p ove-edit`: 13 unit (retime, snap), 7 property tests at 256 cases each (keyframes and source
  frames across split and trim, ripple delete moves exactly the removed length and keeps gaps, extract and insert
  change length by the range, slip/slide/roll invariants, no new overlaps, linked groups stay in sync, and a guard
  that 290–300 of 300 random requests per verb plan successfully so the properties are not vacuous), 15 scenario
  tests, 3 ported trim tests. Every successful plan in the tests is applied, compared with the plan's own result and
  undone with its inverse back to the original. Property run time 0.15 s; `cargo test -p ove-desktop --lib` 75
  passed.

**Group 3 (rebase in merge).**
- Revision records carry an optional `timeMap` (per track, steps `{at, delta}` in microseconds). Old records decode
  unchanged and records without a map write no key (round-trip test plus a decoder fuzz property over truncated and
  corrupted old and new records). The undo of a ripple records the inverse map and its redo the same map, so a file
  printed before an undone ripple is not shifted.
- Rebase is relative: for an item the file changed, each base edge maps through the ripples, and the file edge keeps
  its offset from the base edge. Mapping both edges independently was tried first and failed the property test (an
  item moved across the ripple point was stretched). An item end exactly at an opening stays (it belongs to the item
  before), which a first version also got wrong.
- The store gains `apply_mapped` and `ripples_since(rev)` (reads the records after `rev`); the CLI's and the
  assistant's projection apply call `plan_rebased` with it; the desktop records the map for every planner edit that
  rippled.
- Property "GUI ripple then agent edit equals the sequential result": 10,000 cases (ripple delete, ripple trim or
  insert, then a file that moves and restyles one item and adds one) in 4.9 s; added items keep their times; no
  conflicts. Forced-conflict property: 10,000 cases where a ripple cut items' heads and the file changed their in
  points; every conflict carries `group: "ripple <rev>"` and the same kept side.
- Not done here: a conflict inside a ripple group does not pull the other fields of the group to the same side; the
  group is reported so the user can see the ripple moved them together.

**Group 4 (links).**
- Import (the OS drop onto the window goes through the same import) turns a file with picture and sound into a
  muted video item on Main and an audio item on Audio at the same time with one new `link` value; the sound plays
  once, from the audio item. Files without sound stay one item. Desktop test `importing_a_camera_clip_makes_a_linked_pair`
  checks this with a probed camera file's records (a synthetic probe result: this machine's LGPL FFmpeg build cannot
  make a test file with sound, so no real camera fixture is decoded).
- The editor's move, trim, split and delete edit the link group; move and trim take `linked: false`, sent when Alt
  is held, for J and L cuts. Duplicate and paste give copies their own link group (a copy without its partner is
  unlinked), so copies never join the original's group. Planner property `linked_groups_stay_in_sync` covers move,
  trim, ripple trim and slip; scenario tests cover the override.
- The timeline badges linked items and shows a drift chip (for example `+3f`) on the members of a group that are out
  of sync, computed the same way as the `link.offset` lint. UI test `Links.test.tsx`: an edit outside the planners
  moves the audio of a pair 0.1 s at 30 fps and the chip reads `+3f` on that item.
- The browser mock's import still adds one item per file, so the UI tests' clip counts are unchanged.

