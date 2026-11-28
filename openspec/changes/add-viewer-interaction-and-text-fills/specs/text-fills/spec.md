# Spec Delta

## Purpose

More ways to draw text and vector animation: footage and inverted fills, perspective tilt, per-word styles, and Lottie
items.

## ADDED Requirements

### Requirement: Footage fill
A text item with `textFill` set to `footage` SHALL show the composite below it inside the letters and SHALL fill the
rest of its box with `footageMatteColor`, black by default. Text animations SHALL still apply.

#### Scenario: Video through letters
- **WHEN** a footage-filled title sits over a video clip
- **THEN** the pixels inside the glyphs equal the video frame and the pixels outside equal the matte colour

#### Scenario: Nothing below
- **WHEN** a footage-filled title is the only layer
- **THEN** the glyphs render transparent over the canvas background and no error is raised

### Requirement: Inverted fill
A text item with `textFill` set to `inverted` SHALL show the colour inverse of the composite below inside its letters.

#### Scenario: Inverted over white
- **WHEN** an inverted title sits over a white image
- **THEN** the glyph pixels are black

### Requirement: Text perspective tilt
Text items SHALL accept `transform.rotationX` and `transform.rotationY` between −80° and 80°, rendered as a
perspective tilt about the layer centre, combined with the existing rotation.

#### Scenario: Tilt geometry
- **WHEN** a title has `rotationY` of 45°
- **THEN** its rendered corners lie within 0.5 pixels of the analytic projected corners

#### Scenario: Tilt out of range
- **WHEN** a projection sets `rotationX` to 120
- **THEN** apply refuses the edit with a diagnostic stating the allowed range

### Requirement: Per-word styles
Text and caption items SHALL accept `textSpans`, each giving a word index range and any of colour, weight, style,
background colour and scale. Spans SHALL combine with word animations. A span that falls outside the item's words
after a text change SHALL be dropped on apply, and `check` SHALL warn about it.

#### Scenario: Highlight one word
- **WHEN** a caption has a span colouring word 3 yellow
- **THEN** word 3 renders yellow and the other words keep the caption style

#### Scenario: Span out of range after edit
- **WHEN** a title's text is shortened from 6 words to 3 and a span covered words 5 to 6
- **THEN** apply drops the span and `check` reports a warning naming the item

### Requirement: Lottie items
The app SHALL import Lottie files (JSON and `.lottie`) as media and place them as `lottie` items with `speed`, `loop`,
`holdLast` and colour slot overrides. Frames SHALL be rendered from the vector source at the output size, and the same
frame SHALL render identically every time on both operating systems. Unsupported Lottie features SHALL be reported by
`check`.

#### Scenario: Sharp at any size
- **WHEN** a Lottie item is scaled to 300% and exported at 4K
- **THEN** its edges are as sharp as at 100% (no upscaling blur)

#### Scenario: Hold last frame
- **WHEN** a 2-second Lottie item with `holdLast` is extended to 5 seconds
- **THEN** seconds 2 to 5 show its last frame

#### Scenario: Unsupported feature
- **WHEN** a Lottie file uses expressions
- **THEN** `check` reports a warning naming the item and the unsupported feature, and the rest of the animation
  renders

#### Scenario: Corrupt file
- **WHEN** a corrupt Lottie file is imported
- **THEN** import refuses it with a message and no media entry is added
