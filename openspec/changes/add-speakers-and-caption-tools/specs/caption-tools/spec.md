# Spec Delta

## Purpose

More control over captions: building settings that match the exported layout, subtitle file import and export,
rendered style previews, per-word styles, and translation of caption tracks through the agent.

## ADDED Requirements

### Requirement: Caption building settings
When captions are built from a transcript, the person or agent SHALL be able to set the maximum words, characters and
lines per caption, the longest gap to close between captions (0 to 2 s, 0.5 s by default) and the text case. Captions
SHALL split at sentence ends first, then at clause boundaries, then near the middle. A caption SHALL NOT need more
lines than the maximum when drawn with the chosen style at the project's canvas size.

#### Scenario: Line limit at 9:16 and 16:9
- **WHEN** captions are built with a maximum of 2 lines for a 9:16 project and for a 16:9 project
- **THEN** no caption draws on more than 2 lines in either export

#### Scenario: Short gaps closed
- **WHEN** captions are built with a maximum gap of 0.5 s and two captions are 0.3 s apart
- **THEN** the first caption ends where the second starts

#### Scenario: Impossible limit
- **WHEN** the maximum characters is smaller than the longest single word
- **THEN** that word gets its own caption and the result lists a warning naming it

### Requirement: Subtitle import
The editor SHALL import SRT and WebVTT files as caption items on a new caption track, keeping the file's timing, as one
revision. Markup tags SHALL be removed and HTML entities decoded. Files in UTF-8 with or without a byte-order mark, and
in Windows-1252, SHALL import with correct characters.

#### Scenario: Import keeps timing
- **WHEN** an SRT file with 20 cues is imported
- **THEN** a new caption track has 20 caption items with the cue times and texts

#### Scenario: Broken file
- **WHEN** a subtitle file has a cue with an unreadable time line
- **THEN** the import reports the line number, imports nothing, and creates no revision

### Requirement: Subtitle export
The editor SHALL export a caption track as an SRT or WebVTT file. WebVTT export SHALL optionally include word
timestamps.

#### Scenario: Round trip
- **WHEN** a caption track is exported as SRT and the file is imported again
- **THEN** the new track has the same texts and times to the millisecond

#### Scenario: No caption track
- **WHEN** subtitle export is requested for a project with no caption track
- **THEN** the command refuses with a message that there are no captions to export

### Requirement: Rendered style previews
The caption style gallery SHALL show each style as a short animated preview drawn by the same renderer as export.

#### Scenario: Preview matches export
- **WHEN** a style's gallery preview frame and an exported frame at the same caption time are compared
- **THEN** the two images match within the renderer's golden-frame tolerance

### Requirement: Per-word caption styles
A single word in a caption SHALL be able to have its own colour, scale and emphasis. The word style SHALL appear in
`timeline.tsx` and SHALL be kept by print, apply and merge. Word animations SHALL still apply to a styled word.

#### Scenario: Agent emphasises a keyword
- **WHEN** an agent sets a yellow colour and 1.2 scale on one word through `timeline.tsx` and applies it
- **THEN** that word renders yellow and larger in the preview and the export, and the other words are unchanged

#### Scenario: Style on a word that no longer exists
- **WHEN** a word style refers to a word index past the end of the caption
- **THEN** the apply reports a warning naming the caption and ignores that style

### Requirement: Caption translation
The editor SHALL translate a caption track into another language through the connected agent and write the result as a
new caption track with the same caption timings and the target language set, as one revision. Word highlights SHALL
still move through the translated text.

#### Scenario: Translated track
- **WHEN** a 30-caption English track is translated to Spanish
- **THEN** a new Spanish caption track has 30 captions with the same start and end times

#### Scenario: Agent returns wrong ids
- **WHEN** the agent's answer is missing caption ids after one retry
- **THEN** the command fails with a message naming the missing ids and creates no revision

#### Scenario: No agent connected
- **WHEN** translation is requested and no agent is connected
- **THEN** the command refuses with a message explaining how to connect an agent
