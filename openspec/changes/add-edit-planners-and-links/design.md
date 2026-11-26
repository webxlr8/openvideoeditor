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
