# Spec Delta

## Purpose

Markers on the timeline that people and agents use for notes and for handing work back and forth, with a review
status, ripple behaviour and snapping.

## ADDED Requirements

### Requirement: Marker entities
A project SHALL hold markers. A marker SHALL have an id, a time, a duration (0 for a point), and optional name,
colour, note, status (open, review or resolved; default open) and assignee. A note longer than 4,000 characters SHALL
be refused.

#### Scenario: Add a point marker
- **WHEN** the user presses M with the playhead at 12 s
- **THEN** a point marker with status open is added at 12 s in one revision

#### Scenario: Invalid status refused
- **WHEN** a projection sets a marker status to `done`
- **THEN** the apply is refused with the marker's line and column

### Requirement: Markers in the projection
Markers SHALL print as elements inside the timeline, before the tracks, ordered by time then id, and SHALL round-trip
through print and parse.

#### Scenario: Round trip
- **WHEN** a document with markers is printed, parsed and converted back
- **THEN** the markers are equal to the originals

### Requirement: Marker merge
Concurrent edits to different markers, or to different fields of one marker, SHALL merge without conflict.

#### Scenario: User and agent edit different markers
- **WHEN** the user changes marker m1's colour while an agent applies a file that changes marker m2's status
- **THEN** both changes are kept and no conflict is reported

### Requirement: Markers follow ripples
When a ripple asks markers to follow, markers after the edit SHALL shift with it, a point marker inside a removed range
SHALL be removed, and a range marker overlapping a removed range SHALL shrink.

#### Scenario: Marker after a ripple delete
- **WHEN** 2 s are ripple-deleted before a marker at 20 s with markers following
- **THEN** the marker is at 18 s

#### Scenario: Markers held in place
- **WHEN** the same ripple runs with markers not following
- **THEN** the marker stays at 20 s

### Requirement: Markers as snap targets
Marker start and end times SHALL be snap targets when snapping is on.

#### Scenario: Snap to a marker
- **WHEN** the user drags an item edge within the snap threshold of a marker
- **THEN** the edge lands on the marker time

### Requirement: Review workflow
Agents SHALL be able to list markers by status and assignee, and set a marker to review. A request from an agent to
set a marker to resolved SHALL be refused. Only the user SHALL resolve markers.

#### Scenario: Agent picks up work
- **WHEN** an agent lists markers with status open and assignee agent
- **THEN** it receives those markers with the ids of the items under each one

#### Scenario: Agent tries to resolve
- **WHEN** an agent sets a marker to resolved
- **THEN** the request is refused with code `marker.resolveByAgent` and the marker is unchanged

### Requirement: Marker editing in the GUI
The timeline SHALL show markers on a ruler lane, open an editor popover on click, and filter markers by status.

#### Scenario: Filter by status
- **WHEN** the user filters by review
- **THEN** only markers in review are shown on the lane
