# Spec Delta

## Purpose

Copying a project with all the media it uses into one self-contained project folder that opens on another computer.

## ADDED Requirements

### Requirement: Self-contained package
Packaging SHALL create a new project at a chosen destination that contains the project's history and a copy of every
media file the project references, with every media entry pointing to its copy inside the package. The source project
SHALL NOT change. The package SHALL appear at its destination only when packaging succeeds.

#### Scenario: Opens elsewhere
- **WHEN** a project with media in three external folders is packaged, the package is moved to another computer and
  opened
- **THEN** no media is offline and the timeline renders the same frames as the source project

#### Scenario: Source unchanged
- **WHEN** a project is packaged
- **THEN** the source project's media paths and revision count are unchanged

#### Scenario: Interrupted packaging
- **WHEN** packaging is cancelled half way
- **THEN** nothing exists at the destination path

### Requirement: Deduplication by content
Files with identical content SHALL be copied once, even when they have different names or paths.

#### Scenario: Same file twice
- **WHEN** a project references the same footage file through two different paths
- **THEN** the package contains one copy and both media entries point to it

### Requirement: Free space check
Packaging SHALL estimate the required space and SHALL refuse to start when the destination has less free space.

#### Scenario: Not enough space
- **WHEN** the destination has less free space than the estimate
- **THEN** packaging is refused with a message that states the required and available space, and nothing is copied

### Requirement: Trim to used
With "trim to used", packaging SHALL copy only the used ranges of each video and audio file plus the chosen handles,
and SHALL adjust the items so the timeline shows the same frames.

#### Scenario: Trimmed package
- **WHEN** a project uses 30 s of a 2 h file and is packaged with trim to used and 2 s handles
- **THEN** the packaged media for that file is at most 34 s long, and the frame at each item's start is unchanged

### Requirement: Missing media and report
Media that is offline when packaging SHALL stay referenced and SHALL be listed in a package report, together with the
number of files collected and deduplicated, the bytes copied, and fonts that are used but not included. With "strict",
packaging SHALL fail when any media is missing.

#### Scenario: Missing file reported
- **WHEN** a project with one offline media file is packaged
- **THEN** the package is created, the file is listed as missing in the report, and the item stays offline

#### Scenario: Strict mode
- **WHEN** the same project is packaged with strict
- **THEN** packaging fails with a message naming the missing file, and nothing exists at the destination

### Requirement: Packaging surfaces
Packaging SHALL be available from the app's File menu, the command line and MCP, SHALL run as a background job with
progress and cancel, and SHALL return the report when done.

#### Scenario: Agent packages a project
- **WHEN** an agent calls `package_project` and waits on the returned job
- **THEN** it receives the package path and the report

#### Scenario: Destination exists
- **WHEN** packaging targets a destination that already exists
- **THEN** packaging is refused with a message, and the existing folder is unchanged
