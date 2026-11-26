# Spec Delta

## Purpose

What a person or an agent can export: the settings model, video and audio codecs, containers, alpha, HDR tags,
audio-only output, and rendering a timeline range back into the project.

## ADDED Requirements

### Requirement: Export settings and presets
Export SHALL accept a settings value with container, video codec, profile, bit depth, quality, alpha, audio codec,
sample rate, audio bit depth, time range and output size (a height preset or "match timeline"). Named presets SHALL
expand to a settings value, and the same presets SHALL be available in the app, the CLI and MCP. Invalid combinations
SHALL be refused before any rendering starts, with a message that names the conflicting fields.

#### Scenario: Preset expands to settings
- **WHEN** a person exports with the preset "ProRes 422 HQ master"
- **THEN** the output is a MOV file with ProRes 422 HQ video at the timeline's size and 24-bit PCM audio

#### Scenario: Impossible combination refused
- **WHEN** an export requests ProRes video in an MP4 container
- **THEN** export is refused with a message naming the codec and the container, and no file is written

#### Scenario: Match timeline size
- **WHEN** an export uses size "match timeline" on a 1080×1920 project
- **THEN** the output frame size is 1080×1920

### Requirement: Video codecs within the licence rule
Export SHALL offer H.264, HEVC 8-bit and 10-bit, ProRes 422 Proxy, LT, 422, HQ and 4444, DNxHR LB, SQ, HQ, HQX and
444, VP9, and AV1. HEVC and AV1 SHALL use only encoders provided by the operating system or the graphics vendor. When no
permitted encoder for the requested codec exists on the computer, export SHALL be refused with a message that names the
codec and suggests an available alternative. The job result SHALL name the encoder that was used.

#### Scenario: ProRes master
- **WHEN** a person exports ProRes 422 HQ on macOS and on Windows
- **THEN** both outputs probe as ProRes 422 HQ, 10-bit 4:2:2, and play in a standard player

#### Scenario: HEVC without a hardware encoder
- **WHEN** an HEVC export is requested on a computer that has no permitted HEVC encoder
- **THEN** export is refused before rendering, and the message suggests ProRes or H.264

#### Scenario: Encoder reported
- **WHEN** any video export finishes
- **THEN** the job result names the encoder used and any fallback that happened

### Requirement: Alpha channel export
ProRes 4444 and VP9 exports SHALL carry the composite's alpha channel when alpha is requested; areas with no content
SHALL be transparent. Other codecs SHALL refuse an alpha request.

#### Scenario: Transparent overlay
- **WHEN** a timeline with only a text item is exported as ProRes 4444 with alpha
- **THEN** pixels outside the text have alpha 0 and pixels inside the text have alpha 255

#### Scenario: Alpha with H.264 refused
- **WHEN** an H.264 export requests alpha
- **THEN** export is refused with a message that names the supported alpha codecs

### Requirement: HDR colour tags
When the timeline produces HDR frames, HEVC 10-bit and ProRes exports SHALL be written in 10-bit or more with BT.2020
colour primaries, the selected PQ or HLG transfer, and mastering metadata. When the timeline cannot produce HDR frames,
an HDR export request SHALL be refused with a message.

#### Scenario: HDR tags present
- **WHEN** an HDR timeline is exported as HEVC 10-bit PQ
- **THEN** the output probes with bt2020 primaries, smpte2084 transfer and mastering-display metadata

#### Scenario: HDR not available
- **WHEN** an HDR export is requested and the timeline produces only SDR frames
- **THEN** export is refused with a message that HDR timelines are not available

### Requirement: Audio-only export
Export SHALL write the timeline mix without video as AAC in an M4A file, WAV at 16-bit or 24-bit, or FLAC. The audio
SHALL equal the timeline mix used by preview and video export.

#### Scenario: WAV matches the mix
- **WHEN** a timeline is exported as 24-bit WAV
- **THEN** its samples equal the mixer output for the same range within 1 least significant bit

#### Scenario: Duration of AAC export
- **WHEN** a timeline is exported as M4A
- **THEN** the file duration equals the timeline duration within one AAC frame

### Requirement: Range export
Export SHALL accept a start and end time and render only that range. A range outside the timeline, or with an end not
after its start, SHALL be refused.

#### Scenario: Range only
- **WHEN** a person exports the range 10 s to 20 s
- **THEN** the output is 10 s long and its first frame equals the timeline frame at 10 s

#### Scenario: Bad range refused
- **WHEN** an export requests the range 20 s to 10 s
- **THEN** export is refused with a message, and no job is created

### Requirement: Save range as media
A person or an agent SHALL be able to render a timeline range into the project's media library as one new media item
owned by the project, in one revision. With "replace", the same revision SHALL also replace the items covered by the
range on the chosen tracks with one clip of the new media. Undo SHALL restore the original items. If the rendering
fails, no revision SHALL be created and no partial file SHALL remain.

#### Scenario: Range saved as media
- **WHEN** a person saves the range 5 s to 12 s as media
- **THEN** a new media item of 7 s appears in the library, and the timeline is unchanged

#### Scenario: Render in place
- **WHEN** an agent saves the range 5 s to 12 s as media with replace on all video tracks
- **THEN** the covered items are replaced by one clip of the new media in one revision, and the composite at 8 s is
  visually the same as before (SSIM ≥ 0.98)

#### Scenario: Render fails
- **WHEN** rendering for save range as media fails because a media file is offline
- **THEN** no revision is created, no media is added, and the message names the offline media
