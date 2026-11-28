# Spec Delta

## Purpose

Correct colour from source to output: working and output colour spaces, input detection and override, colour
transforms, log decodes, linear-light compositing, and HDR output and preview.

## ADDED Requirements

### Requirement: Project colour spaces
A project SHALL have a working colour space and an output colour space. The defaults SHALL be Rec.709 for both. A
project with default colour settings SHALL render exactly as before this change.

#### Scenario: Old project unchanged
- **WHEN** a project made before this change is rendered
- **THEN** its frames match the golden frames recorded before this change

#### Scenario: Unknown space name
- **WHEN** a projection sets the working space to a name that is not supported
- **THEN** apply refuses the edit with a diagnostic that lists the supported names

### Requirement: Input colour space detection and override
Each media item SHALL have an input colour space detected from its file tags. The person or an agent SHALL be able to
override it. The override SHALL be one undoable revision.

#### Scenario: HDR source detected
- **WHEN** an HLG phone clip is imported
- **THEN** its input colour space is reported as Rec.2100 HLG

#### Scenario: Untagged log footage
- **WHEN** an untagged S-Log3 clip is imported and its input space is set to S-Log3
- **THEN** the clip renders with normal contrast in a Rec.709 project, and undo restores the previous look

### Requirement: Colour transforms and log decodes
The app SHALL convert between the supported spaces and SHALL decode Apple Log, S-Log3, LogC3, LogC4, V-Log and
Log3G10. A `color-transform` effect SHALL convert an item from one named space to another, with optional tone mapping.

#### Scenario: Round trip identity
- **WHEN** a Rec.709 frame is converted to the working space and back
- **THEN** every channel differs by at most 0.5/1023

#### Scenario: Curve accuracy
- **WHEN** each log curve is evaluated at the code values in its published table
- **THEN** results match the table within 1e-4

### Requirement: Grade and composite order
Grading SHALL happen in the working space's encoding and compositing in linear light. LUTs SHALL declare the space they
expect, Rec.709 by default.

#### Scenario: LUT in its space
- **WHEN** a LUT marked for a log space is applied in a project with an ACEScct working space
- **THEN** the LUT receives values in its declared space and the result matches a reference render

### Requirement: HDR output
When the output space is Rec.2100 PQ or HLG, export SHALL write 10-bit video with the matching transfer, BT.2020
primaries, and mastering display and content light metadata. SDR graphics SHALL be placed at 203 nits.

#### Scenario: PQ tags
- **WHEN** a project with PQ output is exported
- **THEN** the file reports smpte2084 transfer, bt2020 primaries and mastering metadata, and 203-nit white encodes to
  a PQ value of 0.5807

#### Scenario: No 10-bit encoder
- **WHEN** HDR export is requested on a machine with no available 10-bit encoder
- **THEN** export refuses before starting and names the missing capability

### Requirement: HDR preview
Preview SHALL show HDR on displays and systems that support it. On other displays it SHALL show a tone-mapped SDR
version of the same timeline, and HDR export SHALL still be possible.

#### Scenario: SDR display
- **WHEN** an HDR project is opened on an SDR display
- **THEN** the preview is tone-mapped, the diagnostics overlay says so, and export still produces HDR
