# Spec Delta

## Purpose

Mixing and monitoring sound in the editor: volume that changes over time, waveforms and a gain line on clips, level
meters, audio while scrubbing, ducking music under dialogue, loudness-normalised and audio-only export, and extracting
audio from media.

## ADDED Requirements

### Requirement: Volume keyframes
Video and audio items SHALL accept keyframes for `volume`, in percent from 0 to 200, at item-relative times. The heard
gain SHALL be the static volume multiplied by the keyframe value and by any fades. Between two keyframes the gain SHALL
change smoothly in decibels, following the track easing. Preview and export SHALL produce the same gain at the same
timeline time. Volume keyframes SHALL print in `timeline.tsx` and SHALL apply from an edited projection like any other
keyframe track.

#### Scenario: Fade out with keyframes
- **WHEN** an audio item has volume keyframes 100 at 0 s and 0 at 1 s with linear easing, and the timeline is exported
- **THEN** the exported level at 0.5 s is within 0.5 dB of the level halfway between 0 dB and −60 dB, and the item is
  silent from 1 s

#### Scenario: Round trip through the projection
- **WHEN** a document with volume keyframes is printed, parsed and applied without edits
- **THEN** the document is unchanged and no revision is created

#### Scenario: Out-of-range value refused
- **WHEN** an agent applies a projection with a volume keyframe value of 250
- **THEN** the apply is refused with a diagnostic naming the item and the keyframe, and no revision is created

### Requirement: Import analysis cache
After media with audio is imported, the app SHALL analyse it once in the background and keep the result in the
project bundle. Waveform drawing, loudness measurement and later audio analysis SHALL read this result and SHALL NOT
decode the file again while the file is unchanged. A changed or relinked file with different content SHALL be
analysed again.

#### Scenario: One decode per file
- **WHEN** a media file is imported, then its waveform is drawn at three zoom levels and its loudness is measured
- **THEN** the file's audio is decoded exactly once

#### Scenario: Changed file re-analysed
- **WHEN** the content of an analysed media file changes on disk and the project is reopened
- **THEN** the old analysis is not used and a new analysis runs

#### Scenario: Unreadable audio
- **WHEN** a media file's audio stream cannot be decoded
- **THEN** the media shows an analysis error, its clips show no waveform, and playback and editing of other clips
  continue

### Requirement: Clip waveforms
Each clip with sound SHALL show its waveform on the timeline once analysis is ready. The waveform SHALL show only the
part of the source the clip uses, SHALL follow speed and reverse, and SHALL scale with the clip's gain, including
volume keyframes. Clips whose analysis is not ready SHALL show a placeholder, not an empty area.

#### Scenario: Trimmed and reversed clip
- **WHEN** a clip uses seconds 10 to 20 of its source in reverse
- **THEN** its waveform shows source second 20 at the clip's left edge and source second 10 at its right edge

#### Scenario: Gain changes the shape
- **WHEN** a clip's volume is set from 100 to 50
- **THEN** its waveform is drawn at half its previous height without a new analysis

### Requirement: Gain line on clips
Each clip with sound SHALL show a gain line. Dragging the line SHALL change the clip's static volume. Alt-click on
Windows or Option-click on macOS SHALL add a volume keyframe at that point. Dragging a keyframe point SHALL move it in
time and value. One pointer gesture SHALL create exactly one revision.

#### Scenario: Drag the line
- **WHEN** the person drags a clip's gain line down until the readout shows 50 %
- **THEN** the clip's volume is 50, one revision is created, and undo restores the previous volume

#### Scenario: Drag beyond the limit
- **WHEN** the person drags the gain line above 200 %
- **THEN** the value stops at 200 and the readout shows 200 %

### Requirement: Level meters
The editor SHALL show stereo level meters during playback and scrub. Each channel SHALL show its current level, a peak
marker held for 1.5 s, and a clip indicator that stays lit after the signal reaches 0 dBFS until the person clicks it.
The meter SHALL also show true peak and momentary and short-term loudness. Meter readings SHALL match the audio the
person hears within one display frame.

#### Scenario: Known level
- **WHEN** a sine tone at −6 dBFS plays
- **THEN** both channel meters read −6.0 dB within 0.1 dB

#### Scenario: Clip indicator
- **WHEN** the mix reaches 0 dBFS and then becomes quiet
- **THEN** the clip indicator stays lit until the person clicks it

### Requirement: Scrub audio
While the person drags the playhead or scrubs the viewer, the editor SHALL play short pieces of the timeline mix at
the playhead position. Scrub audio SHALL include all tracks, gain and fades as heard in playback. Scrub audio SHALL
stop when the pointer stops. After an edit, scrub audio SHALL reflect the edit.

#### Scenario: Scrub over dialogue
- **WHEN** the person drags the playhead slowly across a dialogue clip
- **THEN** short pieces of the dialogue play, starting within 40 ms of each pointer movement when the area was played
  before, and within 120 ms when it was not

#### Scenario: Scrub after an edit
- **WHEN** the person mutes a track and then scrubs over the same area
- **THEN** the muted track is not heard

### Requirement: Ducking music under dialogue
People and agents SHALL be able to lower chosen music items while chosen dialogue items have speech, with a depth in
decibels, attack, release and hold. Speech SHALL come from transcript word timings when a transcript exists, and from
the dialogue level otherwise. The result SHALL be volume keyframes on the music items, created as one revision.

#### Scenario: Duck under speech
- **WHEN** ducking runs with depth −12 dB on a steady music item under a dialogue item with two lines of speech
- **THEN** the music level during each line is 12 dB lower within 0.5 dB, unchanged within 0.2 dB away from speech,
  and the change is one revision

#### Scenario: No speech found
- **WHEN** ducking runs and the dialogue items contain no speech
- **THEN** no keyframes are written, no revision is created, and the result says that no speech was found

#### Scenario: Unknown item
- **WHEN** ducking names a music item id that does not exist
- **THEN** the call is refused with an error naming the id, and no revision is created

### Requirement: Ducking dry run and repeat runs
A ducking dry run SHALL return the planned keyframes without creating a revision. Running ducking again SHALL replace
earlier keyframes in the affected range, or SHALL combine with existing volume keyframes when asked to combine.

#### Scenario: Repeat run replaces
- **WHEN** ducking runs twice on the same range with depths −12 dB and then −6 dB
- **THEN** the music level during speech is 6 dB lower, not 18 dB

#### Scenario: Dry run
- **WHEN** an agent runs ducking as a dry run
- **THEN** it receives the planned keyframes per music item and the document has no new revision

### Requirement: Loudness-normalised export
Export SHALL offer loudness targets: off, −14, −16, −23 and −24 LUFS integrated. With a target set, the exported mix
SHALL measure within 0.5 LU of the target and SHALL NOT exceed −1 dBTP true peak. With the target off, the mix SHALL
NOT exceed −1 dBTP true peak. The export result SHALL report integrated loudness and true peak before and after
normalisation. People and agents SHALL be able to measure the loudness of the timeline mix without exporting.

#### Scenario: Streaming target
- **WHEN** a timeline measuring −30 LUFS is exported with the −14 LUFS target
- **THEN** the file measures −14.0 LUFS within 0.5 LU, its true peak is at most −1.0 dBTP, and the result lists −30
  and −14 as before and after

#### Scenario: Silent timeline
- **WHEN** a timeline with no audible sound is exported with a loudness target
- **THEN** export completes without applying gain and the result says the mix is silent

### Requirement: Audio-only export
Export SHALL write the timeline mix alone as AAC in an `.m4a` file, as 24-bit WAV, or as FLAC, without rendering
video. The audio-only file SHALL have the timeline duration and the same mix as a video export, including the loudness
target. Audio-only export SHALL follow the same rules as video export for partial files, offline media and pending
items.

#### Scenario: WAV matches the mix
- **WHEN** a timeline is exported as 24-bit WAV
- **THEN** its samples match the mixer output within 1 least significant bit, and its duration equals the timeline
  duration

#### Scenario: Offline media refused
- **WHEN** an audio-only export is started while a clip's media is offline
- **THEN** export is refused with an error naming the clip, and no output file is left behind

### Requirement: Extract audio
People and agents SHALL be able to extract the audio of a media file into the project library as WAV or FLAC. The
extracted file SHALL become a new media entry in one revision.

#### Scenario: Extract from video
- **WHEN** audio is extracted from a video media file as WAV
- **THEN** a new audio media entry appears with the same duration as the source audio, and undo removes the entry

#### Scenario: No audio stream
- **WHEN** audio extraction is requested for a media file that has no audio stream
- **THEN** the call is refused with a message saying the file has no audio, and no revision is created
