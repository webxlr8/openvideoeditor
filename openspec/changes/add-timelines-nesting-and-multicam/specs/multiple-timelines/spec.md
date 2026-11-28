# Spec Delta

## Purpose

Several timelines in one project, each in its own projection file, with nesting that keeps item identity, forks that
merge back, and saving a range as media.

## ADDED Requirements

### Requirement: Timelines in a project
A project SHALL hold one or more timelines, each with an id, name, aspect, frame rate and kind. Opening a project made
before this change SHALL give it one timeline that contains all existing tracks.

#### Scenario: Old project opens
- **WHEN** a project made before this change is opened
- **THEN** it has one timeline `tl_main` with all its tracks and the same aspect, and it renders identically

#### Scenario: Remove a nested timeline
- **WHEN** the user removes a timeline that a nest item uses
- **THEN** the removal is refused and the nest items are listed

### Requirement: One projection file per timeline
The main timeline SHALL print to `timeline.tsx` and every other timeline to `timelines/<id>.tsx`. Applying a file
SHALL only change content inside that file's timeline.

#### Scenario: Scoped apply
- **WHEN** an agent applies `timelines/tl_intro.tsx` with all items removed
- **THEN** only items of `tl_intro` are removed and the main timeline is unchanged

#### Scenario: Existing callers
- **WHEN** a tool prints and applies `timeline.tsx` as before this change
- **THEN** it edits the main timeline with the same results as before

### Requirement: Nest items
A nest item SHALL place one timeline inside another at a time range, with an in point. A nest SHALL refer to an
existing timeline and SHALL not create a cycle; nesting deeper than 8 levels SHALL be refused. Nested content SHALL
appear in preview, agent frames and export.

#### Scenario: Cycle refused
- **WHEN** timeline A nests B and an edit makes B nest A
- **THEN** the edit is refused with code `nest.acyclic`

#### Scenario: Nested content in export
- **WHEN** a project with a nest is exported
- **THEN** the frames inside the nest match the child timeline rendered at the same times

### Requirement: Nest and flatten keep identity
Nesting a selection SHALL move the selected items into a new timeline and replace them with one nest item, keeping
their ids. Flattening SHALL move the items back with remapped times and report nest properties that cannot be kept.
Both SHALL be one revision.

#### Scenario: Marker references survive
- **WHEN** items with ids i_1 and i_2 are nested
- **THEN** the new timeline contains items i_1 and i_2

#### Scenario: Flatten with a look on the nest
- **WHEN** a nest with a colour grade is flattened
- **THEN** the items are back in the parent and a warning names the grade that was not kept

### Requirement: Fork and port back
A named version SHALL be able to fork into a new timeline. The fork's changes SHALL be able to port back to the source
timeline with the project's merge rules, as one revision.

#### Scenario: Port without fork edits
- **WHEN** a fork with no edits is ported back
- **THEN** no revision is created

#### Scenario: Port with a conflict
- **WHEN** the fork and the source both changed the same field of one item
- **THEN** the conflict is reported and resolved by the chosen preference

### Requirement: Save a range as media
The user SHALL be able to save a timeline range as a new media item and optionally replace the range on chosen tracks
with the rendered clip, in one revision.

#### Scenario: Replace a range
- **WHEN** the user saves 10–20 s as media with replace on tracks v1 and v2
- **THEN** the items in that range are replaced by one clip of the rendered media, and undo restores them

### Requirement: Timeline tabs
The editor SHALL show open timelines as tabs, open a nest's timeline on double-click, and keep per-timeline view
state outside the document.

#### Scenario: Open a nest
- **WHEN** the user double-clicks a nest item
- **THEN** its timeline opens in a new tab and no revision is created
