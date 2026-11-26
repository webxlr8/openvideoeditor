# Spec Delta

## Purpose

Shared edit verbs for the timeline: a frame grid, planners that every writer uses, retiming rules, sync lock, safe
merging of concurrent ripples, range selection, tool modes, the two-up viewer, and the command surfaces.

## ADDED Requirements

### Requirement: Frame grid per timeline
Each timeline SHALL have a frame rate. Documents without one SHALL use 30. Planners SHALL snap requested times to the
frame grid. Times off the grid SHALL be accepted on apply and reported by a lint.

#### Scenario: Snapped split
- **WHEN** a split is requested at 1.01 s on a 25 fps timeline
- **THEN** the cut lands at 1.00 s

#### Scenario: Off-grid time from a file
- **WHEN** a projection with an item starting at 1.013 s is applied on a 25 fps timeline
- **THEN** the apply succeeds and verify reports `time.offFrame` for that item

### Requirement: Edit plans
Split, trim, ripple trim, ripple delete, insert, overwrite, lift, extract, close gap, slip, slide and roll SHALL each
produce either a plan or a refusal. Applying a plan SHALL create exactly one revision. A refusal SHALL name a code,
the item ids that caused it and a message, and SHALL change nothing.

#### Scenario: Ripple delete closes the gap
- **WHEN** a 2 s item is ripple-deleted on a sync-locked track
- **THEN** every later item on the sync-locked tracks moves 2 s earlier, in one revision

#### Scenario: Blocked ripple
- **WHEN** a ripple would move an item on a locked track
- **THEN** nothing changes and the refusal has code `ripple.blocked` and lists that item

#### Scenario: Preview does not change the document
- **WHEN** a plan is requested in preview mode
- **THEN** the plan is returned and no revision is created

### Requirement: Retiming respects speed and direction
Every cut or trim SHALL compute source in and out points from the playback speed and direction. Keyframes, caption
segments, fades and transitions SHALL keep their timeline positions across the cut.

#### Scenario: Split at double speed
- **WHEN** an item with speed 2 and in point 0 is split 3 s after its start
- **THEN** the right half's in point is 6 s

#### Scenario: Keyframe survives a split
- **WHEN** an item with an opacity keyframe at timeline time 10 s is split at 8 s
- **THEN** the right half evaluates the same opacity at 10 s as the original did

#### Scenario: Reverse item trim
- **WHEN** the start edge of a reversed item is trimmed later by 1 s
- **THEN** the item shows the same frames at its remaining timeline positions as before

### Requirement: Sync lock
Each track SHALL have a sync lock setting, on by default. A ripple SHALL shift the target track and every sync-locked
track, and any track that holds a link partner of a shifted item.

#### Scenario: Unlocked track stays
- **WHEN** a ripple delete runs while a music track has sync lock off
- **THEN** the music items do not move

### Requirement: Concurrent ripple rebase
Apply SHALL map the times of items edited in a projection through every ripple made after the projection's base
revision, so the item lands where the editor intended.

#### Scenario: Agent edit after a GUI ripple
- **WHEN** an agent prints the timeline, the user ripple-deletes 2 s before item B, and the agent then applies a file
  that changes B's opacity and moves B 1 s later
- **THEN** B ends 1 s later than its rippled position, with the new opacity

#### Scenario: Added items are not mapped
- **WHEN** the same file also adds a new item at 20 s
- **THEN** the new item starts at 20 s

### Requirement: Range selection and tool modes
The editor SHALL support in and out points, and select, blade and trim tool modes, with rebindable keys. In trim mode,
dragging a clip body SHALL slip it, a modifier drag SHALL slide it, and dragging an edit point SHALL roll it.

#### Scenario: Lift and extract a range
- **WHEN** the user sets in at 4 s and out at 6 s and extracts
- **THEN** the range is removed from the targeted tracks and later items move 2 s earlier

#### Scenario: Slip beyond the source
- **WHEN** the user slips an item past the end of its source media
- **THEN** the slip stops at the source limit and the limiting item is outlined

### Requirement: Two-up viewer
During slip, slide and roll the viewer SHALL show the two frames at the affected edit points side by side, and return
to the normal view when the drag ends.

#### Scenario: Slip shows new in and out frames
- **WHEN** the user drags a slip
- **THEN** within 100 ms the viewer shows the new first and last frames of the item

### Requirement: Edit command surfaces
The same verbs SHALL be available from the command line, MCP and the in-app assistant, with a dry-run that returns
the plan, and with refusals returned as structured errors.

#### Scenario: CLI dry run
- **WHEN** `ove edit ripple-delete <project> --track a1 --ranges 12.3-14.0 --dry-run --json` runs
- **THEN** it prints the plan and exits 0, and the project has no new revision

#### Scenario: Stale revision is rebased, not refused
- **WHEN** an agent sends `edit_timeline` with an older `rev` and no conflict exists
- **THEN** the edit applies to the current document as one revision
