# Spec Delta

## Purpose

Separating people from their background on this computer, and using that matte for effect regions, captions and text
behind the subject, and colour readouts of the subject only.

## ADDED Requirements

### Requirement: On-demand mattes
The app SHALL compute a person matte for a video or image media item on request, as a background job with progress and
cancel. The matte SHALL be cached with the project and reused until the media file changes. The model SHALL download
only after the person agrees when it is larger than 50 MB.

#### Scenario: Compute and reuse
- **WHEN** `ove media matte <id>` runs and then a render uses the subject region
- **THEN** the job completes, the render uses the cached matte, and running the command again finishes without
  recomputing

#### Scenario: Consent declined
- **WHEN** the person declines the model download
- **THEN** no download happens, no matte is computed, and features that need the matte show that it is unavailable

#### Scenario: Cancel
- **WHEN** the person cancels a running matte job
- **THEN** the job stops within one second and no partial matte is used

### Requirement: Matte quality
Mattes SHALL reach an intersection over union of at least 0.9 on the held-out portrait test set. Flicker in static
regions (mean frame-to-frame matte change) SHALL be below 0.01.

#### Scenario: Portrait test set
- **WHEN** mattes are computed for the held-out portrait set
- **THEN** mean IoU is at least 0.9 and the flicker metric is below 0.01

### Requirement: Matte view
`ove look --view matte` and a viewer option SHALL show the matte for a time as a grey image.

#### Scenario: Agent sees the matte
- **WHEN** an agent runs `ove look <project> --at 4 --view matte`
- **THEN** the output frame is the matte of the clip at 4 seconds

### Requirement: Effect regions
An effect with `region: subject` SHALL apply only inside the matte, and with `region: background` only outside it.

#### Scenario: Background blur
- **WHEN** a blur with `region: background` is applied to a talking-head clip with a computed matte
- **THEN** the background is blurred and the person is sharp

#### Scenario: Matte missing
- **WHEN** a region effect is used on a clip with no matte yet
- **THEN** the effect applies to the whole frame, verify warns and names the item, and export refuses unless missing
  mattes are allowed explicitly

### Requirement: Captions and text behind the subject
Captions with `behind-subject` and footage-filled text that uses the subject SHALL render between the background and
the person.

#### Scenario: Caption behind person
- **WHEN** a `behind-subject` caption overlaps the person in a clip with a computed matte
- **THEN** the caption is hidden where the person is and visible over the background

### Requirement: Subject-only colour readout
`ove color inspect --region subject` SHALL measure only pixels inside the matte.

#### Scenario: Skin readout
- **WHEN** inspect runs with `--region subject` on a talking-head frame
- **THEN** the readout excludes the background, and the skin-line deviation describes the person only

#### Scenario: Region without matte
- **WHEN** inspect runs with `--region subject` on a clip with no matte
- **THEN** the command exits with code 1 and says that the matte has not been computed
