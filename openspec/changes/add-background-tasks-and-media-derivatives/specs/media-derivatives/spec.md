# Spec Delta

## Purpose

Data derived from media files: a stable identity per file, hover-skim sheets, proxies, and the controls to see and
clear derived data.

## ADDED Requirements

### Requirement: Content identity
Each media file SHALL get a content identity at import, computed from its size and sampled content. Derived data SHALL
be keyed by that identity, so moving, renaming or relinking a file SHALL keep its derived data. A change to the file's
content SHALL invalidate it.

#### Scenario: Relink keeps derived data
- **WHEN** a media file is moved to another folder and relinked
- **THEN** its thumbnails, skim sheet, proxy and transcript are reused without rebuilding

#### Scenario: Content changed
- **WHEN** a media file is replaced by a different file with the same name
- **THEN** its derived data is rebuilt

#### Scenario: Older projects
- **WHEN** a project made before this change is opened
- **THEN** content identities are computed in the background and existing caches are moved to them without loss

### Requirement: Hover-skim
Hovering across a media thumbnail SHALL show frames from across the file in proportion to the pointer position. The
first frame SHALL appear within 16 ms of a pointer move once the skim sheet exists, and skimming SHALL NOT decode video
or send per-frame requests to the core.

#### Scenario: Skim a clip
- **WHEN** the person moves the pointer across a video thumbnail in the media panel
- **THEN** the thumbnail shows frames from start to end following the pointer

#### Scenario: Sheet not built yet
- **WHEN** the person hovers a file whose skim sheet is still queued
- **THEN** the normal thumbnail stays and the sheet's task moves to interactive priority

### Requirement: Proxies
For sources at or above 4K, 10-bit long-GOP sources, or sources that fail a real-time decode check, the app SHALL be
able to build a 1080p proxy. When "Use proxies" is on for the project, playback, skim and analysis SHALL use the proxy.
Export and agent inspection frames SHALL always use the original media.

#### Scenario: Smooth 4K playback
- **WHEN** a 4K 10-bit HEVC clip with a proxy plays with "Use proxies" on
- **THEN** playback drops no frames on the reference machines

#### Scenario: Export uses originals
- **WHEN** a project with proxies is exported
- **THEN** the export decodes the original files, and the output matches an export with proxies off

#### Scenario: Proxy missing
- **WHEN** "Use proxies" is on and a proxy is not built yet
- **THEN** the original plays and the proxy task is queued

### Requirement: Storage controls for derived data
Settings → Storage SHALL show the size of each kind of derived data (thumbnails, skim sheets, proxies, transcripts,
analysis) for the open project and in total, with a Clear action per kind. Clearing SHALL never touch original media or
the project document.

#### Scenario: Clear proxies
- **WHEN** the person clears proxies for a project
- **THEN** the proxy files are deleted, the space is shown as freed, and playback uses originals until proxies rebuild

#### Scenario: Clear while in use
- **WHEN** the person clears skim sheets while a sheet task is running
- **THEN** the task is cancelled first and then the files are removed
