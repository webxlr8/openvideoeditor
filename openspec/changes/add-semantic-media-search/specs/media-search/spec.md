# Spec Delta

## Purpose

Finding moments in footage by what they show and what is said, on this computer; using the results as timeline clips;
and organising media as document data that people and agents edit together.

## ADDED Requirements

### Requirement: Footage indexing
When visual search is installed and enabled, each video and image in the project SHALL be indexed in the background.
Videos SHALL be split into shots at frame-accurate cuts, and each shot SHALL be represented by at least one sampled
frame, with one more frame for every 8 seconds of shot length. Near-black and flat frames SHALL be skipped. The index
SHALL be stored in the project bundle and SHALL be reused after relinks.

#### Scenario: Index a new file
- **WHEN** a video is imported into a project with visual search enabled
- **THEN** an indexing task is queued and the media panel shows its progress until the file is indexed

#### Scenario: Cut accuracy
- **WHEN** a test video made of clips with known cut points is indexed
- **THEN** every reported shot boundary is within 2 frames of a true cut

#### Scenario: Model not installed
- **WHEN** a project is opened and the visual model is not installed
- **THEN** nothing is indexed, nothing is downloaded, and the media panel offers to install it with its size

#### Scenario: Corrupt index file
- **WHEN** an index file is damaged
- **THEN** it is discarded and rebuilt, and search skips that file until it is rebuilt

### Requirement: Visual search
A text query SHALL return shots ranked by relevance, each with the media name, shot range, best-frame time, a suggested
window of up to 4 seconds around the best frame inside the shot, and a confidence between 0 and 1. Only the best frame
of each shot SHALL count. Results below the confidence threshold SHALL be omitted. A frame SHALL also be usable as the
query ("more like this").

#### Scenario: Find a shot
- **WHEN** the person searches "boat in a harbour" in a project that contains such a shot
- **THEN** that shot is among the first 5 results with its best frame shown

#### Scenario: More like this
- **WHEN** the person chooses "Find similar" on a frame
- **THEN** shots that look similar are listed, and the source shot itself is first

#### Scenario: Nothing relevant
- **WHEN** no shot passes the confidence threshold
- **THEN** the Moments section says no visual matches, and other sections still show their results

### Requirement: Spoken search
A query SHALL match transcribed speech with case and accents ignored. Quoted text SHALL match as an exact phrase.
Results SHALL be ranked, and each hit SHALL give the word range from the first word's start to the last word's end,
plus 150 ms on each side.

#### Scenario: Phrase match
- **WHEN** the person searches `"the price is"` and one clip contains those words in that order
- **THEN** the hit's range starts at "the" and ends after "is", padded by 150 ms

#### Scenario: Untranscribed media
- **WHEN** some media has no transcript
- **THEN** spoken search covers only transcribed media and says how many files are not transcribed, with a
  Transcribe action

### Requirement: Search in the media panel
The media panel SHALL offer one search field with result sections Moments, Spoken, Timelines and Files, and scope
chips to limit them. Results SHALL appear within 300 ms after typing stops for a project with 10 hours of indexed media.
Dragging a Moment or Spoken result onto the timeline SHALL place a clip that uses the result's range.

#### Scenario: Drag a result
- **WHEN** the person drags a Moment result onto a track
- **THEN** a clip is created whose source in and out are the result's suggested window, as one undoable revision

#### Scenario: Index status
- **WHEN** indexing is running
- **THEN** the panel shows "Indexing n of N" and results include only indexed files

### Requirement: Organisation as document data
Media SHALL be organisable into folders (nested by path), tags, saved subclips (name, in, out) and smart folders (a
saved query and scope). Every change SHALL be a revision through the normal apply pipeline, so it merges with
concurrent edits and undoes in one step. Folder paths SHALL appear in the projection header's media table.

#### Scenario: Batch organise
- **WHEN** an agent moves 20 files into two new folders and renames 3 files in one call
- **THEN** the change is one revision and one undo restores the previous state

#### Scenario: Save a result as a subclip
- **WHEN** the person chooses "Save as subclip" on a search result
- **THEN** the subclip appears under its media with the result's range and can be dragged like media

#### Scenario: Concurrent edits
- **WHEN** the person renames a folder while an agent tags files in it
- **THEN** both changes are kept

#### Scenario: Smart folder
- **WHEN** the person saves the query "interview" as a smart folder
- **THEN** the folder lists current matches and updates when new media is indexed

### Requirement: Agent and CLI tools
Search, transcripts, overview sheets and organisation SHALL be available from the CLI and MCP, also when the app is not
running. Each search hit SHALL include a clip fragment that can be pasted into the timeline projection unchanged. A
search tool SHALL report index health (not installed, indexing with counts, ready) when that explains missing results.
No tool SHALL start a model download larger than the consent limit.

#### Scenario: Paste a hit
- **WHEN** an agent pastes a hit's clip fragment into a track in `timeline.tsx` and applies it
- **THEN** the check passes and the clip plays the hit's range

#### Scenario: Headless search
- **WHEN** an agent runs the media search command on a closed project with an existing index
- **THEN** results are returned without starting the app

#### Scenario: Model missing for an agent
- **WHEN** an agent calls visual search and the model is not installed
- **THEN** the result has no visual hits, reports "model not installed" with the size, and nothing is downloaded

#### Scenario: Overview sheet
- **WHEN** an agent asks for an overview of a 10-minute video
- **THEN** it receives one sheet of at most 36 tiles with time labels, built from the shot list without decoding the
  whole file again
