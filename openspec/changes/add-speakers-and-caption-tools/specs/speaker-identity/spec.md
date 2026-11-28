# Spec Delta

## Purpose

Knowing who speaks when: on-device speaker detection, consistent speakers across all files of a project, a project
speaker list that people and agents can edit, speaker runs in the transcript, and speaker-tinted waveforms.

## ADDED Requirements

### Requirement: Speaker detection on this computer
The editor SHALL detect speakers in the speech of each media file without sending audio off the computer. Each
recognised word SHALL get a speaker label.

#### Scenario: Two speakers found
- **WHEN** speaker detection runs on a 10-minute recording of two people talking in turns
- **THEN** two speakers are found and the diarization error rate is at most 15%

#### Scenario: Works offline
- **WHEN** speaker detection runs with no network connection and the model already downloaded
- **THEN** detection completes and no network request is made

#### Scenario: Model not installed
- **WHEN** speaker detection is requested and the model is not downloaded
- **THEN** the editor asks for consent to download it, shows its size, and does nothing until the person agrees

### Requirement: Consistent speakers across files
The same person SHALL get the same speaker in every media file of a project. Matching SHALL NOT depend on the order in
which files were analysed.

#### Scenario: Same person in three files
- **WHEN** one person speaks in three separate recordings of a project
- **THEN** all three recordings label that person with the same speaker

#### Scenario: Order does not matter
- **WHEN** the same files are analysed in two different orders
- **THEN** both runs give the same speaker for each word

### Requirement: Project speaker list
The project SHALL keep a list of speakers with a name and a colour. Renaming, recolouring, merging two speakers and
splitting a speaker in one media file SHALL each be one revision that can be undone. The speaker list SHALL appear in
the `timeline.tsx` header, and editing a name there and applying SHALL rename the speaker.

#### Scenario: Agent renames a speaker
- **WHEN** an agent changes speaker "S2" to "Host" in the `timeline.tsx` header and applies it
- **THEN** the speaker list shows "Host", History shows one revision, and Undo restores "S2"

#### Scenario: Merge two speakers
- **WHEN** the person merges speaker "S3" into "S1" for one media file
- **THEN** all words of "S3" in that file are labelled "S1" and other files are unchanged

#### Scenario: Unknown speaker id
- **WHEN** a merge names a speaker id that does not exist
- **THEN** the command refuses, names the id, and creates no revision

### Requirement: Speakers in the transcript and on the timeline
The timeline transcript SHALL include speaker runs. Audio waveforms on the timeline SHALL be tinted with the speaker's
colour where that speaker talks, when speaker tint is turned on.

#### Scenario: Speaker runs
- **WHEN** an agent reads the transcript of a two-speaker interview
- **THEN** the response lists runs of consecutive words with their speaker id and name

#### Scenario: Recolour
- **WHEN** the person changes a speaker's colour
- **THEN** the waveform tint changes to the new colour on every clip of that speaker
