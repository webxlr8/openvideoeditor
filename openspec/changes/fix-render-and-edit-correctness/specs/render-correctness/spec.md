# Spec Delta

## Purpose

Blend modes, split and trim timing, export encoder availability, generation job resume and audio button labels behave
as the schema and their labels state.

## ADDED Requirements

### Requirement: Every schema blend mode renders
Each of the 16 `blendMode` values the schema accepts SHALL render with its own blending result. No accepted value
SHALL render as Normal unless it is `normal`. The value `add` SHALL keep rendering as additive blending.

#### Scenario: Difference blend
- **WHEN** a white layer with `blendMode` `difference` covers a frame of colour (0.8, 0.4, 0.1)
- **THEN** the rendered pixel is (0.2, 0.6, 0.9) within one 8-bit step

#### Scenario: Colour dodge is not add
- **WHEN** a layer with colour 0.25 and `blendMode` `color-dodge` covers a backdrop of 0.4
- **THEN** the rendered value is 0.4 / 0.75 within one 8-bit step, not 0.65

### Requirement: Split keeps every frame in place
Splitting a clip at a time inside it SHALL produce two parts that together show the same source frame, the same
animated property values and the same caption words at every timeline time as the clip before the split, for any
playback speed and for reverse playback. The split SHALL be one revision.

#### Scenario: Split a clip at double speed
- **WHEN** a clip from 2 s to 6 s with in-point 10 s and speed 2 is split at 3 s
- **THEN** the right part starts at 3 s with in-point 12 s and the left part keeps in-point 10 s

#### Scenario: Split a reversed clip
- **WHEN** a reversed clip from 0 s to 4 s with in-point 10 s is split at 1 s
- **THEN** the right part has in-point 10 s and the left part has in-point 13 s

#### Scenario: Keyframes survive a split
- **WHEN** a clip whose opacity goes linearly from 0 at 0 s to 1 at 4 s is split at 1 s
- **THEN** both parts have opacity 0.25 at the split time and the right part reaches 1 at its end

#### Scenario: Split outside the clip is refused
- **WHEN** a split is requested at the clip's start, end or outside it
- **THEN** the request is refused with "Move the playhead inside the clip to split it." and no revision is made

### Requirement: Split gives edge settings to their side
Fade in, audio fade in and transition in SHALL stay on the left part of a split. Fade out, audio fade out and
transition out SHALL move to the right part. The right part's id SHALL NOT equal any id in the document.

#### Scenario: Fades after a split
- **WHEN** a clip with a fade in and a fade out is split
- **THEN** the left part has only the fade in, the right part has only the fade out, and the right part has a new id

### Requirement: Trim keeps animation and captions in place
Moving a clip's start edge SHALL keep its keyframes and caption segments at the same timeline times; keyframes and
segments before the new start SHALL be cut at it. For reverse playback, moving the end edge SHALL move the in-point,
and extending the start edge SHALL be limited by the source length.

#### Scenario: Trim the start of an animated clip
- **WHEN** a clip from 0 s to 4 s with a keyframe at 2 s has its start moved to 1 s
- **THEN** the keyframe is stored at 1 s from the new start, so it still plays at 2 s on the timeline

#### Scenario: Trim the end of a reversed clip
- **WHEN** a reversed clip from 10 s to 20 s with in-point 2 s has its end moved to 17 s
- **THEN** its in-point becomes 5 s

### Requirement: Export has a good H.264 encoder on every supported machine
The shipping FFmpeg build SHALL contain the NVIDIA, Intel and AMD hardware H.264 and HEVC encoders on Windows, the
platform encoders on both systems, and a software H.264 encoder, all under licences the project allows. Export SHALL
only fall back to MPEG-4 Part 2 when none of these opens, and SHALL then show the existing fallback warning.

#### Scenario: Hardware encoder unavailable
- **WHEN** an export runs on a Windows machine whose GPU encoders and Media Foundation encoder all fail to open
- **THEN** the export uses the software H.264 encoder and produces an H.264 MP4 without the fallback warning

#### Scenario: Licence gate
- **WHEN** the FFmpeg build finishes
- **THEN** the licence check passes and reports no GPL or non-free component

### Requirement: Paid generations survive a restart
A generation job whose worker stopped while the provider still holds the request SHALL resume and fill its placeholder
without submitting the request again. A job that made no progress within the timeout, counted from its first submission,
SHALL fail and remove its placeholder as before.

#### Scenario: App closed during a video generation
- **WHEN** the app closes while a video generation is running at the provider and is opened again
- **THEN** the job resumes, the result fills the placeholder, and the provider received one submission

#### Scenario: Stored address on a foreign domain
- **WHEN** a resumed job's stored status address is not on the provider's own domains
- **THEN** the job fails with a refusal and the API key is not sent to that address

### Requirement: Audio buttons say what they do
The audio panel buttons that set music to a fixed level and reset volume SHALL be labelled with what they do until
real ducking and loudness normalisation exist.

#### Scenario: Lower music button
- **WHEN** the person opens the Audio panel
- **THEN** the buttons read "Lower music to 30 %" and "Reset volumes to 100 %", and no button is labelled "Duck"
