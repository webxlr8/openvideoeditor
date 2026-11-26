# Spec Delta

## Purpose

Working directly in the viewer: knowing where every layer is on screen, selecting by clicking, transforming and
cropping with handles, guides and safe areas, a full-screen viewer and playback rates.

## ADDED Requirements

### Requirement: Layer geometry
For any project time the app SHALL give the on-screen corners, crop corners and stacking order of each visible layer.
`ove geometry --at <t>` and an MCP tool SHALL return the same data as JSON. The geometry SHALL match the rendered
frame.

#### Scenario: Geometry matches render
- **WHEN** points are sampled across a frame with rotated, cropped and tilted layers
- **THEN** for at least 99.5% of points, a layer's quad contains the point exactly when that layer's rendered alpha at
  the point is above zero

#### Scenario: Time past the end
- **WHEN** geometry is requested for a time after the timeline ends
- **THEN** the command exits with code 1 and states the timeline duration

### Requirement: Click to select
Clicking in the viewer SHALL select the topmost layer whose rendered pixels are under the pointer. Clicking where no
layer is drawn SHALL clear the selection.

#### Scenario: Transparent area of a top layer
- **WHEN** the person clicks on a transparent part of a title that lies over a video clip
- **THEN** the video clip is selected, not the title

### Requirement: Transform and crop handles
When a layer is selected and playback is paused, the viewer SHALL show handles to move, scale, rotate and crop it, and
to tilt it when it is text. Dragging SHALL preview live. Releasing SHALL create exactly one revision. Handles SHALL
snap to canvas centre and edges, and rotation SHALL snap to 15° steps while Shift is held.

#### Scenario: One revision per drag
- **WHEN** the person drags a layer across the viewer and releases
- **THEN** one revision is created with the new position, and undo restores the old position

#### Scenario: Crop a rotated layer
- **WHEN** the person drags the right crop handle of a layer rotated by 30°
- **THEN** the crop moves along the layer's own right edge

#### Scenario: Drag cancelled
- **WHEN** the person presses Esc during a drag
- **THEN** the layer returns to its position before the drag and no revision is created

#### Scenario: Handles hidden in playback
- **WHEN** playback starts
- **THEN** handles are hidden until playback pauses

### Requirement: Guides and safe areas
The viewer SHALL offer grids (thirds and 2 to 5 divisions), action-safe and title-safe areas, a centre cross and format
masks (2.39:1, 1.85:1, 1:1, 9:16, 4:5) with editable platform presets. Guides SHALL never appear in exports.
`ove look --overlay` SHALL render the chosen guides into the agent frame.

#### Scenario: Guides not exported
- **WHEN** guides are on and the project is exported
- **THEN** the exported frames contain no guide pixels

#### Scenario: Agent overlay
- **WHEN** `ove look <project> --at 3 --overlay safe,grid` is run
- **THEN** the frame shows the safe areas and grid over the composited image

### Requirement: Safe-area lint
Verify SHALL warn when a text or caption item extends outside the title-safe area for any part of its duration, naming
the item and the time.

#### Scenario: Title outside safe area
- **WHEN** a title is placed so its right edge is 2% from the frame edge
- **THEN** verify reports a title-safe warning for that item

#### Scenario: Title inside safe area
- **WHEN** all text and captions stay inside title-safe
- **THEN** verify reports no title-safe warning

### Requirement: Full-screen viewer
The app SHALL show the viewer full screen with floating controls for play, scrub, timecode, loop, rate and guides. The
controls SHALL hide after 2 seconds without pointer movement. Esc SHALL leave full screen.

#### Scenario: Enter and leave
- **WHEN** the person enters full screen and presses Esc
- **THEN** the viewer fills the screen, then returns to the editor layout as it was

### Requirement: Playback rates
Playback SHALL run at 0.5×, 1×, 1.25×, 1.5× and 2×. Audio SHALL play resampled at these rates. The rate SHALL also be
settable over the control link by agents.

#### Scenario: Rate change
- **WHEN** the person selects 1.5× and plays for 10 seconds of wall time
- **THEN** the playhead advances 15 seconds of timeline time within one frame

#### Scenario: Unsupported rate
- **WHEN** an agent requests a rate of 7× over the control link
- **THEN** the request is refused with the list of supported rates
