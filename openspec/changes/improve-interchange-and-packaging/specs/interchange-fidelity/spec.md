# Spec Delta

## Purpose

Timeline exchange that keeps framing, opacity, audio levels, keyframes and source timecode, writes FCPXML correctly for
the chosen editor, round-trips OTIO without loss, and reports what will not carry before writing.

## ADDED Requirements

### Requirement: Extended properties in exchange
Timeline export to FCPXML, FCP7 XML and OTIO SHALL carry each clip's position, scale, rotation, horizontal flip, crop,
opacity, volume, fades, keyframes for these properties, enabled state and source timecode. Import from the same formats
SHALL read them back.

#### Scenario: Framing carried
- **WHEN** a clip scaled to 120 % and moved 200 px right is exported as FCP7 XML and opened in Premiere Pro
- **THEN** the clip appears at 120 % scale, 200 px right of centre

#### Scenario: Volume carried
- **WHEN** an audio clip at −6 dB with a 1 s fade-in is exported as FCPXML for Final Cut Pro
- **THEN** Final Cut Pro shows the clip at −6 dB with a 1 s fade-in

### Requirement: FCPXML target and version
FCPXML export SHALL let the person choose the target editor (DaVinci Resolve or Final Cut Pro) and the format version
(1.10 to 1.14). The written values SHALL make the clip appear with the same framing, crop, speed and keyframe timing in
the chosen editor. A version the target does not accept SHALL be refused with a message.

#### Scenario: Resolve framing
- **WHEN** a timeline with a scaled, rotated and cropped clip is exported for DaVinci Resolve and rendered there
- **THEN** the rendered frame matches our export of the same frame with SSIM ≥ 0.95

#### Scenario: Audio-only use of a video source
- **WHEN** a timeline uses only the sound of a video file and is exported for each target
- **THEN** the target editor plays the sound and shows no picture from that file

#### Scenario: Unsupported version refused
- **WHEN** an export requests a version the chosen target does not accept
- **THEN** export is refused with a message that lists the accepted versions

### Requirement: Lossless OTIO round trip
Exporting a timeline as OTIO and importing it back SHALL give the same items with the same fields, including fields the
format does not define. When the extra data is missing from an OTIO file, import SHALL use the format's own fields and
SHALL report which properties were not restored.

#### Scenario: Round trip
- **WHEN** a timeline with effects, captions, keyframes and colour grades is exported as OTIO and imported into a new
  project
- **THEN** the imported items equal the originals, field by field, apart from new ids

#### Scenario: Extra data stripped
- **WHEN** an OTIO file whose extra data was removed by another tool is imported
- **THEN** cuts and media are imported, and the report lists the properties that were not restored

### Requirement: Import tolerances
Export followed by import in the same format SHALL return extended property values within these tolerances: position
±0.5 px, scale ±0.1 %, rotation ±0.01°, volume ±0.1 dB, keyframe times ±1 frame.

#### Scenario: FCPXML round trip within tolerance
- **WHEN** a timeline is exported as FCPXML for Resolve and imported back
- **THEN** each extended property is within its tolerance

### Requirement: Fidelity report per target
Before writing, timeline export SHALL list each property used in the timeline that the chosen format, target and
version will not carry, with a count and example item names. The report SHALL be shown in the app, printed by the command
line, and returned to agents.

#### Scenario: Report lists effects
- **WHEN** a timeline with two blur effects is exported as FCP7 XML
- **THEN** the report lists "effects: 2 items" with their names before the file is written

#### Scenario: Nothing lost
- **WHEN** a timeline with only cuts, scale and volume is exported as FCPXML for Final Cut Pro
- **THEN** the report states that nothing will be lost
