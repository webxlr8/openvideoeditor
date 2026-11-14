# Spec Delta

## Purpose

Guarantees that keep the app usable, keep the user's files and keys safe, and never hide a loss of content.

## ADDED Requirements

### Requirement: Contained failures
An internal failure while handling any user, command-line, MCP or assistant request SHALL be reported as a plain-language
error and SHALL NOT close the app, stop the command-line control link or disable later edits.

#### Scenario: Internal failure in an editor command
- **WHEN** an editor command fails internally while a project is open
- **THEN** the user sees an error message, the app stays open, and the next edit to the project succeeds

#### Scenario: Internal failure in an MCP request
- **WHEN** one MCP request fails internally
- **THEN** that request gets an error response and the next request on the same connection is answered

### Requirement: Export never destroys an earlier file
Export SHALL replace an existing file at the chosen path only after the new file is complete.

#### Scenario: Export fails over an existing file
- **WHEN** the user exports to a path that already holds a file and the export fails or is cancelled
- **THEN** the earlier file is unchanged and no partial file is left beside it

#### Scenario: Offline media
- **WHEN** the timeline uses a media file that is offline
- **THEN** export is refused with a message naming the offline media

### Requirement: Keys never reach logs
API keys SHALL NOT appear in log files or console output at any log level.

#### Scenario: Debug logging with a stored key
- **WHEN** debug logging is enabled and the assistant or a generation provider makes a request with a stored key
- **THEN** the log output contains no part of the key

### Requirement: Keys only go to their provider
A provider key SHALL only be sent over HTTPS to that provider's own API hosts.

#### Scenario: Provider response names another host
- **WHEN** a generation provider's response asks the app to poll or download from a host outside the provider's API hosts
- **THEN** the app sends that request without the key, or refuses it

### Requirement: HDR sources look right in SDR
PQ and HLG sources SHALL be tone-mapped to BT.709 SDR the same way in the viewer, in `look` frames and in export.

#### Scenario: HLG phone clip
- **WHEN** an HLG clip from a phone is placed on the timeline
- **THEN** preview and export show it with SDR contrast and colour, and the two match within the golden tolerance

### Requirement: No silent loss in interchange
Timeline export to another editor SHALL list, before writing, every item kind and property that will not carry over.

#### Scenario: Motion graphics in the timeline
- **WHEN** a timeline with captions and a motion graphic is exported as FCPXML
- **THEN** the dialog lists the captions and the motion graphic as not carried over before the user continues

### Requirement: The UI does not promise missing features
Templates, buttons and presets SHALL describe only what the app does.

#### Scenario: A panel fails to render
- **WHEN** one panel throws while rendering
- **THEN** that panel shows an error card with a reload action and the rest of the editor keeps working
