# Spec Delta

## Purpose

Editing video through what people say: a timeline transcript with stable word ids, removing words and dead air,
filler candidates, transcript search, dry-run previews, and the surfaces (CLI, MCP, assistant, Transcript panel) that
expose them.

## ADDED Requirements

### Requirement: Timeline transcript
The editor SHALL produce a transcript of the timeline: every recognised word of every audible clip, in timeline order,
with its text, timeline start and end, the clip it belongs to, and a word id. Timeline times SHALL account for each
clip's trim, speed and direction. The response SHALL include the project revision it was computed from. The same
project revision SHALL give the same transcript from the app, the CLI, MCP and the assistant.

#### Scenario: Words follow trim and speed
- **WHEN** a clip with source words at 10.0 s and 12.0 s is trimmed to start at source 9.0 s, placed at timeline 0 s
  and played at 2× speed
- **THEN** the transcript lists those words at timeline 0.5 s and 1.5 s

#### Scenario: Same result on every surface
- **WHEN** the transcript of one project revision is read through the CLI, MCP and the assistant
- **THEN** the three word lists are identical

#### Scenario: Media not transcribed yet
- **WHEN** the transcript is requested and a clip's media has no transcript
- **THEN** the response lists that media as untranscribed and transcribes it only if the request asks for it

### Requirement: Stable word ids
Each word id SHALL identify the source media and the source time of the word. A word id SHALL stay the same when other
parts of the timeline are edited, cut or moved.

#### Scenario: Ids survive a cut elsewhere
- **WHEN** words are removed from one clip
- **THEN** the ids of all words in other clips, and of the remaining words in that clip, are unchanged

#### Scenario: Id no longer on the timeline
- **WHEN** a removal names a word id whose word is no longer on the timeline
- **THEN** the command refuses that id, names it in the error, and changes nothing else in that request

### Requirement: Remove words
The editor SHALL remove selected words from the timeline, given as word ids, id ranges or exact text matches, and close
the gaps on the affected tracks. All cuts of one request SHALL be one revision and one undo step.

#### Scenario: Word removed, neighbours kept
- **WHEN** one word in the middle of a sentence is removed and the result is exported and transcribed again
- **THEN** the removed word is absent and both neighbouring words are present

### Requirement: Natural cut edges
Each word cut SHALL keep part of the natural pause on each side according to the chosen pause setting (tight, balanced
or loose; balanced by default). Cut edges SHALL be placed at a quiet point near the word boundary without cutting into
neighbouring words, and each seam SHALL get a short audio fade.

#### Scenario: Pause setting changes the gap
- **WHEN** the same word is removed once with the tight setting and once with the loose setting
- **THEN** the joined pause is shorter with the tight setting than with the loose setting

#### Scenario: No click at the seam
- **WHEN** a word is removed from speech recorded over room noise
- **THEN** the peak level in the 5 ms around each seam is at most −40 dBFS above the room-noise level

### Requirement: Word cuts are atomic
A word removal request SHALL either apply completely as one revision or change nothing.

#### Scenario: One undo step
- **WHEN** ten separate words are removed in one request and the person presses Undo once
- **THEN** the timeline is exactly as before the request

#### Scenario: Locked track blocks the cut
- **WHEN** a removal would need to shift items on a locked track
- **THEN** the command refuses, names the locked track and the blocking item, and creates no revision

### Requirement: Captions follow cuts
When a cut removes words covered by a caption item, the caption item SHALL drop those words and keep the timing of the
remaining words aligned with the audio. A caption item with no remaining words SHALL be removed. This SHALL happen in
the same revision as the cut.

#### Scenario: Caption shortened
- **WHEN** two words in the middle of a caption are removed
- **THEN** the caption shows the remaining words, each highlighted when it is heard

### Requirement: Remove dead air
The editor SHALL find pauses using on-device voice activity detection. A pause SHALL count as dead air only when its
level is at least a set amount below the speaker's own speech level (12 dB by default) and it lasts at least the
minimum pause (0.5 s by default, settable from 0.25 s to 3 s). A padding (0.15 s by default, settable from 0 to 0.5 s)
SHALL be kept on each side of each cut, except at the edges of a file or clip. Removal SHALL be one revision.

#### Scenario: Silence removed, music bed kept
- **WHEN** dead air is removed from speech, 2 s of silence, speech, 4 s of music at −20 LUFS, and speech
- **THEN** the silence is removed and the music is kept

#### Scenario: Detection only
- **WHEN** dead-air detection is requested without removal
- **THEN** the ranges are returned per media file in source time and the document is unchanged

#### Scenario: No speech found
- **WHEN** dead-air removal runs on a clip with no detected speech
- **THEN** the command removes nothing and reports that no speech was found

### Requirement: Filler candidates
The editor SHALL offer filler candidates: words in a per-language filler list and voiced sounds that no recognised word
covers, up to 1.2 s long. Context-dependent fillers SHALL be flagged but not selected by default. Fillers SHALL be cut
only when the person or agent confirms the selection.

#### Scenario: Review before cutting
- **WHEN** the person chooses "Remove fillers"
- **THEN** a list of candidates is shown with each one playable, and nothing is cut until the person confirms

#### Scenario: Verbatim recognition
- **WHEN** a media file is transcribed with the verbatim option
- **THEN** the cached transcript is stored separately from the normal transcript and both stay available

### Requirement: Transcript search
The editor SHALL search all transcribed media of a project for a phrase, ignoring case and accents. Each hit SHALL give
the media, the word-accurate source start and end, and every place that source range is used on the timeline. The
response SHALL say how many media files are not transcribed yet.

#### Scenario: Phrase across a caption boundary
- **WHEN** a search for a three-word phrase matches words that span two caption items
- **THEN** one hit is returned with the start of the first word and the end of the last word

#### Scenario: Accent-insensitive
- **WHEN** a search for "cafe" runs on a transcript containing "café"
- **THEN** the word is returned as a hit

### Requirement: Dry-run preview
Every cutting command SHALL accept a dry-run option that returns a summary of the planned cuts and the resulting
change to `timeline.tsx` as a diff, without creating a revision.

#### Scenario: Preview then apply
- **WHEN** a removal is requested as a dry run and then the same request is applied without other edits in between
- **THEN** the applied change equals the previewed diff

### Requirement: Transcript surfaces
The transcript functions SHALL be available as CLI commands, MCP tools and assistant tools with the same arguments and
results, and in the Transcript panel. In the panel the person SHALL be able to select words, see them struck through,
and cut them with Delete; the panel SHALL follow playback and seek when a word is clicked.

#### Scenario: Cut from the panel
- **WHEN** the person selects three words in the Transcript panel and presses Delete
- **THEN** the words are cut from the timeline as one revision and the panel shows the new transcript

#### Scenario: CLI works without the app
- **WHEN** `ove edit remove-words` runs on a project while the app is closed
- **THEN** the cut is applied to the bundle as one revision and appears in History when the project is opened
