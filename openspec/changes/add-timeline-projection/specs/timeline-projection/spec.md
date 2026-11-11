# Spec Delta

## Purpose

The timeline as a text file: printing the document as a literal TSX projection, checking and applying edited
projections back onto the live document without losing concurrent edits, referencing media from the projection,
placeholders, verify lints, and the CLI commands that expose these.

## ADDED Requirements

### Requirement: Deterministic projection
Printing SHALL produce `timeline.tsx` and `timeline.d.ts` in the bundle from the current document. The same document
SHALL always print byte-identically. Tracks print in document order; items within a track print by start time, then
id. The projection SHALL be rewritten after every new revision.

#### Scenario: Stable output
- **WHEN** the same document is printed twice, on macOS and on Windows
- **THEN** both outputs are byte-identical, with LF line endings

#### Scenario: Rewritten after an edit
- **WHEN** a batch is applied from any source
- **THEN** `timeline.tsx` reflects the new revision and its header `rev` equals the new rev

### Requirement: Projection header
The projection SHALL begin with a comment header listing aspect ratio, duration, track and item counts, a media table
(name, id, type, duration, resolution, status), cast, current diagnostics, pending count and `rev`. The header is
informational: edits to it SHALL be ignored on apply except `rev`.

#### Scenario: Header edits ignored
- **WHEN** a projection is applied after changing the duration line in its header
- **THEN** the document duration is unchanged and no error is raised

### Requirement: Tag and attribute mapping
Each item SHALL print as its tag (`<clip>` for video; `image`, `audio`, `text`, `caption`, `code`, `adjustment`, `shader`
by type) inside its track element, with attribute renames `start`↔`startTime`, `end`↔`endTime`, `in`↔`trimStart`,
`out`↔`trimEnd`, `kind`↔`type`. Fields equal to their defaults SHALL be omitted.

#### Scenario: Round trip
- **WHEN** any valid document is printed, parsed and converted back
- **THEN** the result equals the original document after normalisation (fields equal to their defaults removed)

### Requirement: Literal-only syntax
Attribute values SHALL be literals: strings, numbers, booleans, arrays and objects of literals. Expressions,
identifiers, spreads, function calls and template literals with substitutions SHALL be errors. Comments SHALL be
dropped. Quoted string attributes keep backslashes literally; escapes require a braced string.

#### Scenario: Expression rejected
- **WHEN** a projection contains `start={2 + 3}`
- **THEN** check reports an error at that line and column saying only literal values are allowed

#### Scenario: Backslash in quoted attribute
- **WHEN** a projection contains `textContent="C:\new"`
- **THEN** the stored text is exactly `C:\new` (backslash and `n`), not a newline

### Requirement: Diagnostics
Check SHALL report every problem it finds with severity (error or warning), line, column, code and message. Errors
SHALL refuse the whole apply. Warnings SHALL NOT block apply and SHALL be listed in the next printed header. Unknown
attributes on items SHALL round-trip unchanged without a warning; unknown keys inside known object attributes SHALL
warn.

#### Scenario: Multiple errors
- **WHEN** a projection has a syntax error on line 10 and an invalid enum value on line 40
- **THEN** check reports both with positions, and apply changes nothing

#### Scenario: Unknown key in a known object
- **WHEN** an item has `transform={{x: 0, scael: 2}}`
- **THEN** check warns about `scael` and apply keeps the rest of the transform

### Requirement: Item identity
Items with an `id` that exists in the base revision SHALL be treated as edits of that item. Items without an `id`
SHALL be created with a new id. Items present in the base revision but missing from the file SHALL be removed. An item
whose `id` changed SHALL be treated as removing the old item and adding a new one.

#### Scenario: Removal is reported
- **WHEN** a projection omits 3 items that exist in its base revision
- **THEN** apply removes them and reports "removed 3 items" with their ids and names

#### Scenario: New item gets an id
- **WHEN** a projection contains an item without `id`
- **THEN** apply creates it and the next projection shows its assigned id

### Requirement: Three-way merge onto the live document
Apply SHALL compute the changes between the file and its base revision (named by the header `rev`) and apply them onto
the current document item by item and field by field. Changes made since the base revision to items or fields the
file did not change SHALL be kept. The result SHALL be one new revision.

#### Scenario: Concurrent edits to different items
- **WHEN** the GUI moves item A after the projection was printed, and the file then changes item B
- **THEN** after apply both A's move and B's change are present

#### Scenario: Concurrent edits to different fields of one item
- **WHEN** the GUI changes item A's opacity and the file changes item A's start time from the same base
- **THEN** after apply item A has both the new opacity and the new start time

#### Scenario: Unchanged file applied after GUI edits
- **WHEN** a projection is applied unchanged after the GUI has edited items since it was printed
- **THEN** no revision is created and the GUI edits are kept

#### Scenario: Same file applied twice
- **WHEN** an edited projection is applied, then applied again without changes
- **THEN** the second apply creates no revision and reports no conflicts

### Requirement: Merge granularity
Merge SHALL compare item fields individually, and object-valued fields (such as transform, crop, colour grading) one
key deeper. Array-valued fields (such as keyframes, caption segments, effects) SHALL be merged as whole values.

#### Scenario: Different keys of one object
- **WHEN** the GUI changes `transform.x` and the file changes `transform.scale` of the same item from the same base
- **THEN** after apply the item has both changes

### Requirement: Conflict policy
When the file and the current document both changed the same field of the same item since the base revision, the
file's value SHALL win by default; with `prefer editor` the current value SHALL be kept. Every conflict SHALL be
reported with item id, field, file value and editor value.

#### Scenario: Same field, default policy
- **WHEN** both sides changed item A's `end` from the same base
- **THEN** the file's value is applied and the conflict is reported

#### Scenario: Same field, prefer editor
- **WHEN** the same conflict is applied with `prefer editor`
- **THEN** the editor's value is kept and the conflict is reported

### Requirement: Removal conflicts
When one side removed an item that the other side changed since the base revision, the result SHALL follow the policy:
under the default policy the file's intent wins (the item is removed, or re-created with its id); under `prefer
editor` the current state wins. Each case SHALL be reported as a conflict.

#### Scenario: File removes an item the editor changed
- **WHEN** the GUI changed item A's opacity and the file omits item A, from the same base
- **THEN** item A is removed under the default policy, kept under `prefer editor`, and a conflict is reported

#### Scenario: File edits an item the editor removed
- **WHEN** the GUI deleted item A and the file changes item A's start, from the same base
- **THEN** item A is re-created with its id under the default policy, stays deleted under `prefer editor`, and a
  conflict is reported

### Requirement: Base revision availability
Apply SHALL refuse a projection whose `rev` is missing, malformed, newer than the current revision, or no longer
retained, with a message to print the timeline again.

#### Scenario: Pruned base
- **WHEN** a projection's `rev` refers to a revision that has been pruned
- **THEN** apply is refused (exit code 1) and nothing changes

### Requirement: Text-edit apply
Apply SHALL also accept a list of `{oldText, newText}` edits against the current printed projection. Each `oldText`
SHALL occur exactly once and edits SHALL NOT overlap; `newText: ""` deletes; the edited text is then applied as a
projection with the printed `rev` as base. A whole replacement source SHALL also be accepted.

#### Scenario: Ambiguous edit
- **WHEN** an edit's `oldText` occurs twice in the projection
- **THEN** apply is refused with the edit index and both match positions, and nothing changes

### Requirement: Media references
An item's `src` SHALL resolve, in order, to a media name from the header table, a media id, or a file path (absolute,
`./`, `../`, `~/`, relative to the bundle). A path not yet in the registry SHALL be added as a new media entry awaiting
probe, and the printed projection SHALL show its media name. An unresolvable `src` SHALL be an error.

#### Scenario: New file path
- **WHEN** a projection adds `<clip src="~/Footage/take3.mov" … />` for a file that exists
- **THEN** apply registers the file as media, the item references its id, and the next projection shows its name

#### Scenario: Missing file
- **WHEN** `src` points to a path that does not exist
- **THEN** check reports an error naming the path

### Requirement: Pending placeholders
Items MAY be marked `pending` with a duration and a description. Pending items SHALL be counted in the header and by
verify. Filling a pending item (setting its media and clearing `pending`) SHALL keep its id, position and duration
unless the fill changes them.

#### Scenario: Fill keeps id
- **WHEN** a pending item is filled with generated media
- **THEN** its id is unchanged and the pending count drops by one

### Requirement: Verify
Verify SHALL report findings with severity, item ids and a message, and SHALL be extensible with lints from later
changes. This change provides: overlapping items on one track, gaps on the first video track, clips extending past
their source duration (holding the last frame), missing or offline media, and pending items.

#### Scenario: Clean project
- **WHEN** verify runs on a project with no findings
- **THEN** it exits 0 and reports zero findings

#### Scenario: Pending items block a clean verify
- **WHEN** a project has 2 pending items
- **THEN** verify reports both and exits 1

### Requirement: CLI commands
The CLI SHALL provide `source print [--from <line>] [--lines <n>]`, `source check [file] [--prefer file|editor]`,
`source apply [file] [--prefer file|editor]`, `verify`, `history` and `undo [rev]`, each taking a project name, id or
bundle path and `--json`, exiting 0 on success and 1 when refused, with errors on stderr and results on stdout.

#### Scenario: JSON output
- **WHEN** `source apply <project> --json` succeeds
- **THEN** stdout is one JSON object with the new rev, counts of added/updated/removed items, conflicts and warnings

#### Scenario: Undo to a revision
- **WHEN** `undo <project> <rev>` is run
- **THEN** the document returns to that revision's state as a new revision and exit code is 0

### Requirement: Projection performance
For a 5,000-item project, print SHALL take ≤ 100 ms and check + apply of a one-item edit SHALL take ≤ 150 ms on the
reference machines.

#### Scenario: Large project apply
- **WHEN** one attribute is changed in the projection of a 5,000-item project and applied
- **THEN** the apply completes in ≤ 150 ms
