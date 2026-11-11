# Spec Delta

## Purpose

The project as stored and changed: bundles in a library, the canonical document and its schema, the operations that
change it, revisions and undo, and the durability and locking guarantees that keep it from being lost or corrupted.

## ADDED Requirements

### Requirement: Project library location
Projects SHALL live in a library folder, by default `~/Movies/<App>/` on macOS and `%USERPROFILE%\Videos\<App>\` on
Windows, overridable in settings and by an environment variable. A library on a network volume SHALL be refused with
a plain-language reason; media referenced from network volumes SHALL be allowed.

#### Scenario: Default library
- **WHEN** the app or CLI needs the library and none is configured
- **THEN** it creates and uses the platform default folder

#### Scenario: Network library refused
- **WHEN** the user sets the library to a folder on an SMB/NFS/AFP volume
- **THEN** the change is refused with a message explaining that projects must be on a local drive

### Requirement: Project bundle creation
Creating a project SHALL create a bundle folder `<Name>.<ext>` in the library containing `project.json`, and the
`media/`, `cache/`, `proxies/`, `.revs/` and `.scratch/` folders.

#### Scenario: New project layout
- **WHEN** a project named `Promo` is created in an empty library
- **THEN** `Promo.<ext>/` exists with `project.json` and the five folders, and `project.json` passes validation

### Requirement: Bundle folder names
Bundle folder names SHALL be sanitised: `\ / : * ? " < > |` and control characters removed, no leading dots, no
trailing dots or spaces, at most 120 characters, "Untitled Project" when empty, a suffix for Windows reserved device
names (CON, PRN, AUX, NUL, COM1–9, LPT1–9), and a numeric suffix when the name already exists.

#### Scenario: Name with forbidden characters
- **WHEN** a project is created with the name `Q3: "Launch" / Final?`
- **THEN** the bundle folder is `Q3 Launch  Final.<ext>` (forbidden characters removed) and the document name is
  stored unchanged

#### Scenario: Name collision
- **WHEN** a project named `Promo` is created and `Promo.<ext>` already exists
- **THEN** the bundle is `Promo 2.<ext>` and the existing bundle is untouched

### Requirement: Project library operations
The library SHALL list projects with id, name, created/updated time, aspect ratio, duration and item count without
loading full documents, and SHALL support rename (document name only; the folder never changes), duplicate (new
project id, same content, media copied only if owned by the bundle) and move to the system Trash / Recycle Bin.

#### Scenario: Rename keeps the folder
- **WHEN** a project is renamed
- **THEN** its document name changes and its bundle folder path does not

#### Scenario: Delete goes to Trash
- **WHEN** a project is deleted
- **THEN** its bundle is moved to the system Trash / Recycle Bin and disappears from the list

#### Scenario: Listing a broken bundle
- **WHEN** a bundle in the library has an unreadable or invalid `project.json`
- **THEN** the list shows it as damaged with the reason, and other projects list normally

### Requirement: Canonical document schema
`project.json` SHALL contain `format`, `version`, `project` (id, name, createdAt, updatedAt, stateUpdatedAt, aspectRatio,
media, timelineItems, tracks) and `media` (mediaId, nativePath, ownedByBundle), with item and track fields as in
docs/spec/document-model.md. Unknown fields SHALL be preserved on read and write.

#### Scenario: Unknown fields round-trip
- **WHEN** a document containing fields this version does not know is opened, edited and saved
- **THEN** the unknown fields are written back unchanged

#### Scenario: Invalid document
- **WHEN** `project.json` fails schema validation
- **THEN** open fails with the path of the first invalid field, and the file is not modified

### Requirement: Schema versioning and migration
Documents with an older `version` SHALL be migrated forward on open, after copying the original to
`.revs/pre-migration-<version>.json`. Documents with a newer `version` than the app supports SHALL open read-only with
a message to update the app.

#### Scenario: Older version
- **WHEN** a version-1 document is opened by an app that writes version 2
- **THEN** a pre-migration copy is kept and the document opens migrated

#### Scenario: Newer version
- **WHEN** a document has a version newer than the app supports
- **THEN** it opens read-only and no write is attempted

### Requirement: Document operations
The document SHALL change only through batches of the 10 op kinds (`addTrack`, `updateTrack`, `removeTrack`,
`reorderTracks`, `unsetTrackFields`, `addItem`, `updateItem`, `removeItems`, `unsetItemFields`, `setAspectRatio`). A
batch SHALL apply entirely or not at all.

#### Scenario: Atomic batch
- **WHEN** a batch of 5 ops is applied and the 4th violates an invariant
- **THEN** none of the 5 ops take effect, and the error names the op index and the violated invariant

### Requirement: Document invariants
After every batch: track and item ids SHALL be unique; every item SHALL reference an existing track and, where it has
media, an existing media entry; `startTime < endTime`; times, trims and durations SHALL be finite and non-negative;
track `order` values SHALL be unique and contiguous; enum fields SHALL hold known values.

#### Scenario: Item on a missing track
- **WHEN** a batch adds an item whose `trackId` does not exist
- **THEN** the batch is rejected with the invariant `item.track.exists`

#### Scenario: Removing a track with items
- **WHEN** a batch removes a track that still has items
- **THEN** the batch is rejected unless the same batch removes or moves those items

### Requirement: Revisions
Every applied batch SHALL produce a new revision whose `rev` is strictly greater than the previous one, recorded with
time, source (gui, cli, mcp, agent, migration) and summary. The current `rev` SHALL be readable without loading the
document. Any recorded revision SHALL be restorable, and restoring SHALL itself be a new revision.

#### Scenario: Monotonic rev
- **WHEN** two batches are applied within the same millisecond
- **THEN** the second revision's `rev` is greater than the first

#### Scenario: Restore
- **WHEN** the user restores revision R
- **THEN** the document equals the document at R and a new revision with source `restore` is recorded

### Requirement: Undo and redo
Undo SHALL revert the most recent batch from the current session's undo stack and redo SHALL reapply it; each is a new
revision. A batch applied from any source SHALL be one undo step. A new batch after an undo SHALL clear the redo stack.

#### Scenario: Undo an agent edit
- **WHEN** an agent applies a batch of 40 ops and the user presses Undo once
- **THEN** all 40 ops are reverted together

### Requirement: Durable saves
A batch SHALL be reported as applied only after it is durably on disk. A crash or power loss at any moment SHALL leave
the project openable at either the state before or after the last batch, never a mix or an empty file.

#### Scenario: Kill during save
- **WHEN** the process is killed at a random point while writing a batch (repeated 1,000 times in tests)
- **THEN** every reopen yields the previous or the new state, and never a corrupt document

#### Scenario: Disk full
- **WHEN** the disk fills during a save
- **THEN** the batch is rejected with a "disk full" error and the previous state remains intact

### Requirement: Single writer
At most one process SHALL hold a project open for writing. Another process trying to open it for writing SHALL get a
"project is open in <process>" error; a lock left by a dead process SHALL be detected and taken over.

#### Scenario: Second writer
- **WHEN** the app has a project open and another process opens it for writing directly
- **THEN** the second open fails with an error naming the holder

#### Scenario: Stale lock
- **WHEN** the lock holder crashed and a new process opens the project
- **THEN** the stale lock is detected, the project opens, and the takeover is logged

### Requirement: Media registry
The document SHALL record each media entry's id, source path, whether the bundle owns the file, its probe status
(awaiting-probe, probed, failed) and its last known size and modification time. On open, entries whose file is missing
or changed SHALL be marked offline or changed without modifying the document.

#### Scenario: Missing media on open
- **WHEN** a project is opened and a referenced file has been moved
- **THEN** the project opens, the entry is reported offline, and `project.json` is unchanged

### Requirement: Document performance
On the reference machines, opening a document with 5,000 items SHALL take ≤ 200 ms, and applying a 1-op batch to it
SHALL take ≤ 10 ms including the durable write (p95).

#### Scenario: Large project
- **WHEN** a generated 5,000-item project is opened and 1,000 single-op batches are applied
- **THEN** open ≤ 200 ms and apply p95 ≤ 10 ms
