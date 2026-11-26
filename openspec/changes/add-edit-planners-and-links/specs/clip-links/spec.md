# Spec Delta

## Purpose

Link groups between items, such as a clip's picture and its separate sound, so they edit together by default, with an
override for J and L cuts, and lints that report broken groups.

## ADDED Requirements

### Requirement: Link groups
Items SHALL be able to share a link value. Items with the same link value SHALL form one group. Moves, trims, slips,
splits and deletes SHALL apply to the whole group by default.

#### Scenario: Move a linked pair
- **WHEN** the user drags a video item linked to an audio item 1 s later
- **THEN** both items move 1 s later in one revision

#### Scenario: Delete a linked pair
- **WHEN** the user deletes the video item of a linked pair
- **THEN** the audio item is deleted too

### Requirement: Edit one side
A request SHALL be able to edit one member of a group without the others. The GUI SHALL offer this with a modifier
key.

#### Scenario: L cut
- **WHEN** the user trims the end of the audio item of a linked pair with the modifier held
- **THEN** only the audio item changes and the group remains

### Requirement: Link lints
Verify SHALL report a group with one member as `link.single`, and a group whose members show different source offsets
as `link.offset` with the drift in frames. The timeline SHALL show the drift on the affected items.

#### Scenario: Drift after a file edit
- **WHEN** an agent applies a file that moves only the audio item of a linked pair 3 frames later
- **THEN** the apply succeeds and verify reports `link.offset` of 3 frames

#### Scenario: Orphaned link
- **WHEN** an agent applies a file that removes the audio item of a linked pair
- **THEN** the apply report lists the remaining item and verify reports `link.single`

### Requirement: Links on import
Importing or dropping a media file with picture and sound onto the timeline SHALL create a linked video item and audio
item.

#### Scenario: Import a camera clip
- **WHEN** the user drops a camera file with sound onto an empty timeline
- **THEN** one video item and one audio item appear with the same link value and the same timing

### Requirement: Links in the projection
Link values SHALL print as an item attribute and SHALL merge per item like other fields.

#### Scenario: Agent links two items
- **WHEN** an agent adds the same link attribute to two items in the file and applies it
- **THEN** the two items edit together from then on
