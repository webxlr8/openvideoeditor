# Spec Delta

## Purpose

Named versions of a project that survive history pruning, can be compared and restored, and undo limited to one actor
or one timeline.

## ADDED Requirements

### Requirement: Save a named version
The user and agents SHALL be able to save the current revision as a named version with an optional note. A saved
version SHALL survive history pruning. A duplicate name SHALL be refused.

#### Scenario: Version survives pruning
- **WHEN** a version is saved and later 3,000 revisions are made
- **THEN** the version can still be printed and restored

#### Scenario: Duplicate name
- **WHEN** a second version is saved with an existing name
- **THEN** the request is refused with code `version.exists`

### Requirement: Compare versions
Comparing two versions, or a version and the current revision, SHALL list items and markers that were added, removed,
moved, trimmed or otherwise changed.

#### Scenario: Diff after a trim
- **WHEN** an item was trimmed by 1 s between two versions
- **THEN** the diff lists that item as trimmed with the old and new times

### Requirement: Restore a version
Restoring a version SHALL create one new revision whose document equals the version. The restore SHALL be undoable.

#### Scenario: Restore and undo
- **WHEN** the user restores version "client v2" and then undoes
- **THEN** the document returns to the state before the restore

### Requirement: Undo scoped to an actor
Undo SHALL be able to target only the revisions of one actor: the user, one agent session or one assistant turn. It
SHALL revert those revisions as one new revision and keep every other actor's changes. If another actor later changed
the same fields, the undo SHALL be refused and list the affected items.

#### Scenario: Agent undoes its own edit
- **WHEN** the user edits item A, an agent edits item B, the user edits item C, and the agent asks to undo its changes
- **THEN** only the change to B is reverted and the changes to A and C remain

#### Scenario: Conflict refuses undo
- **WHEN** the user changed the same field of B after the agent
- **THEN** the agent's undo is refused with code `undo.conflict` naming B

### Requirement: Versions in the GUI and CLI
The History panel SHALL show saved versions with compare and restore actions. The command line SHALL offer save, list,
print, diff and restore.

#### Scenario: CLI diff
- **WHEN** `ove version diff <project> "v1" "v2" --json` runs
- **THEN** it prints the classified changes and exits 0
