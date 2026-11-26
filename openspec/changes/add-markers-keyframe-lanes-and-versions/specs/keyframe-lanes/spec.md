# Spec Delta

## Purpose

Keyframes that people can see and edit on the timeline, with easing per keyframe, a hold easing, more properties, and
merging per property.

## ADDED Requirements

### Requirement: Easing per keyframe
A keyframe SHALL be able to set its own easing, which overrides the track easing for the segment that starts at it.
The easings SHALL include hold, which keeps the value until the next keyframe.

#### Scenario: Hold keyframe
- **WHEN** an opacity track has keyframes 1.0 at 0 s with hold and 0.0 at 2 s
- **THEN** opacity is 1.0 at 1.9 s and 0.0 at 2 s

### Requirement: Keyframable properties
Keyframes SHALL be supported for position, scale, rotation, opacity, blur, radius, crop edges and volume.

#### Scenario: Crop keyframes
- **WHEN** left crop has keyframes 0 at 0 s and 0.25 at 1 s with linear easing
- **THEN** left crop is 0.125 at 0.5 s in preview and export

### Requirement: Merge per property
Concurrent edits to keyframes of different properties on one item SHALL merge without conflict. Concurrent edits to the
same property SHALL follow the existing conflict policy.

#### Scenario: Different properties
- **WHEN** the user edits opacity keyframes while an agent applies a file that edits scale keyframes on the same item
- **THEN** both edits are kept with no conflict

#### Scenario: Same property
- **WHEN** both edit opacity keyframes
- **THEN** a conflict is reported and resolved by the chosen preference

### Requirement: Keyframe lanes
Each track SHALL expand to show lanes for keyframed properties of its items. The user SHALL be able to add, drag and
delete keyframes in a lane. Each gesture SHALL be one revision. Lane expansion SHALL not change the document.

#### Scenario: Drag a keyframe
- **WHEN** the user drags an opacity keyframe from 1 s to 1.5 s
- **THEN** the keyframe moves to 1.5 s in one revision

#### Scenario: Expanding a lane
- **WHEN** the user expands a lane
- **THEN** no revision is created

### Requirement: Keyframe commands
The command line and MCP SHALL set, add and remove keyframes for one property of one item. A keyframe outside the
item's time range SHALL be refused.

#### Scenario: Set from the CLI
- **WHEN** `ove edit keyframes <project> --item i_1 --property opacity --set 0:1,2:0` runs
- **THEN** the item has those two opacity keyframes in one revision

#### Scenario: Keyframe outside the item
- **WHEN** a keyframe is set 5 s after the item ends
- **THEN** the command exits 1 with code `keyframe.outOfRange`
