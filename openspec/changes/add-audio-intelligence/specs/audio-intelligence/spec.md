# Spec Delta

## Purpose

Local audio analysis that edits depend on: reading source timecode, synchronising recordings by timecode or by sound
with drift correction, detecting beats and downbeats for cutting and snapping, and removing noise from speech with a
blend amount.

## ADDED Requirements

### Requirement: Source timecode
Probing media SHALL read embedded start timecode from video files that carry a timecode track and from WAV files that
carry Broadcast WAV or iXML timecode metadata. The timecode SHALL keep its exact frame rate, including fractional
rates and drop-frame. Media without timecode SHALL report no timecode.

#### Scenario: Video and recorder share timecode
- **WHEN** a video file starting at 10:00:00:00 at 23.976 drop-frame and a recorder WAV starting 4 seconds later are
  probed
- **THEN** both report their start timecode, and their difference is 4 seconds to the exact sample

#### Scenario: No timecode
- **WHEN** a phone video without a timecode track is probed
- **THEN** the media reports no timecode and probing succeeds

### Requirement: Audio sync
People and agents SHALL be able to align target items to a reference item. With mode `auto`, timecode SHALL be used
when both items have it, and sound otherwise. Sound alignment SHALL place items to within 1 ms of the true offset when
the recordings overlap by at least 3 seconds. Placement SHALL NOT be rounded to whole frames. The plan SHALL be applied
as one revision.

#### Scenario: Sync by sound
- **WHEN** a recorder item whose sound starts 12.345 s after the camera's sound is synced to the camera item
- **THEN** the recorder item is placed with an offset error of at most 1 ms, the method is reported as sound, and one
  revision is created

### Requirement: Sync dry run and refusals
A sync dry run SHALL return each target's offset, confidence, method and drift without creating a revision. Targets
that cannot be matched with enough confidence, or whose placement would overlap another item, SHALL be skipped and
reported, not placed.

#### Scenario: Dry run
- **WHEN** an agent runs sync as a dry run
- **THEN** it receives offset, confidence, method and drift per target, and the document has no new revision

#### Scenario: Unrelated recordings
- **WHEN** a target item with unrelated sound is synced to the reference
- **THEN** the target is not moved, the result reports it as unmatched with its confidence, and the command exits with
  a non-zero code when no target was placed

#### Scenario: Placement would overlap
- **WHEN** the synced position of a target overlaps another item on the same track
- **THEN** that target is not moved and the result names the item in the way

### Requirement: Drift correction
When synced recordings are longer than 150 seconds, sync SHALL measure clock drift between them. Drift between 20 and
500 parts per million SHALL be corrected by changing the playback speed of the audio-only target, never of a video
item. The result SHALL report the drift in parts per million and the remaining error in milliseconds. Drift outside
that range SHALL be reported and not applied.

#### Scenario: Recorder runs fast
- **WHEN** a two-hour recorder item that runs 50 ppm fast is synced to a camera item
- **THEN** the reported drift is 50 ppm within 3 ppm, the recorder item's speed is corrected, and the offset error is
  at most 1 ms at the start and at most 2 ms at the end

#### Scenario: Implausible drift
- **WHEN** the measured drift is 2,000 ppm
- **THEN** no speed change is applied and the result reports the drift as out of range

### Requirement: Beat detection
People and agents SHALL be able to detect beats, downbeats, tempo and bars in music media or items. For an item, beat
times SHALL be reported both in source seconds and in timeline seconds, with trim, speed and reverse applied. Results
SHALL be cached and SHALL NOT be written into the project document. Detection SHALL run on the computer without a
network connection once the model is installed.

#### Scenario: Steady tempo
- **WHEN** beats are detected on a 120 BPM click track
- **THEN** beats are reported every 500 ms within 20 ms and the tempo is 120 BPM within 0.5 BPM

#### Scenario: Three-four time
- **WHEN** beats are detected on music in three-four time
- **THEN** a downbeat is reported every third beat

#### Scenario: Retimed item
- **WHEN** beats are requested for an item that plays its source at 2× speed starting at timeline second 10
- **THEN** timeline beat times are spaced half as far apart as the source beat times and start from timeline second 10

#### Scenario: Model not installed
- **WHEN** beats are requested before the beat model is installed
- **THEN** the call returns a "model not installed" status with the download size and does not download without
  consent

### Requirement: Beats as snap targets
When "Snap to beats" is on, moving or trimming an item on the timeline SHALL snap to beats of music items, with
downbeats preferred over other beats when both are within reach. Beat marks SHALL be visible on music items while the
option is on.

#### Scenario: Snap to a downbeat
- **WHEN** the person drags a clip edge near a downbeat of a music item with "Snap to beats" on
- **THEN** the edge lands exactly on the downbeat's timeline time

#### Scenario: Option off
- **WHEN** "Snap to beats" is off
- **THEN** beats are not snap targets and no beat marks are drawn

### Requirement: Denoise with blend amount
Video and audio items SHALL accept a denoise amount from 0 to 100. With an amount above 0, the item's sound SHALL be a
blend of the original and a cleaned version, weighted by the amount, aligned to the sample so that no echo or comb
effect is heard. The cleaned version SHALL be produced in the background; until it is ready the item SHALL play its
original sound and show progress. The amount SHALL print in `timeline.tsx` and SHALL be undoable.

#### Scenario: Speech in noise
- **WHEN** an item with speech in steady noise at 5 dB signal-to-noise ratio gets denoise amount 100
- **THEN** the noise-only parts are at least 20 dB quieter and the speech level changes by at most 1 dB

#### Scenario: Partial amount
- **WHEN** the denoise amount is 50
- **THEN** the item's sound equals the average of the original and the cleaned sound, and the two are aligned within
  one sample

#### Scenario: Not ready yet
- **WHEN** the person plays an item whose cleaned version is still being produced
- **THEN** the original sound plays and the item shows denoise progress

### Requirement: Export never ships un-cleaned sound silently
Export SHALL produce any missing cleaned version before writing the file. If producing a cleaned version fails, export
SHALL stop with an error naming the item, unless the person or agent explicitly allows unprocessed sound for that
export.

#### Scenario: Cleaned version missing
- **WHEN** export starts while one item's cleaned version has not been produced
- **THEN** export produces it first, shows that step in its progress, and the exported sound is cleaned

#### Scenario: Cleaning fails
- **WHEN** producing an item's cleaned version fails during export
- **THEN** export stops with an error naming the item and no output file is left behind

#### Scenario: Explicitly allowed
- **WHEN** the same export runs with unprocessed sound allowed
- **THEN** export completes with that item's original sound and the result lists the item as unprocessed
