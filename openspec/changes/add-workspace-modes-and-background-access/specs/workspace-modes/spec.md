# Spec Delta

## Purpose

How the workspace adapts to the task: layout presets, splitters, maximise, the full-screen viewer and playback rates,
the aspect-mismatch prompt, onboarding, local feedback with a screenshot, what's-new slides, a local activity view and
the light theme.

## ADDED Requirements

### Requirement: Layout presets and splitters
The editor SHALL offer the layout presets Edit, Assistant, Review and Vertical in the View menu. Panels SHALL resize
with draggable splitters, and each preset SHALL remember its own sizes. A key SHALL maximise the focused panel and a
second press SHALL restore it.

#### Scenario: Sizes kept per preset
- **WHEN** a person widens the chat in Assistant, switches to Edit and back to Assistant
- **THEN** the chat has the width the person set

#### Scenario: Maximise
- **WHEN** the timeline has focus and the person presses the maximise key twice
- **THEN** the timeline fills the window and then returns to its previous size

#### Scenario: Vertical suggestion
- **WHEN** a person opens a 9:16 project for the first time
- **THEN** the app suggests the Vertical preset once, and declining keeps the current layout

### Requirement: Full-screen viewer
The person SHALL open a full-screen viewer that shows only the video and floating controls (scrub, timecode, loop,
playback rate, safe-area guides). The controls SHALL hide after 2 seconds without pointer motion. Esc SHALL exit.

#### Scenario: Enter and exit
- **WHEN** a person presses the Toggle Fullscreen chord and then Esc
- **THEN** the viewer fills the screen with controls, and Esc returns to the editor at the same playhead

### Requirement: Playback rates
Playback SHALL offer 0.5×, 1×, 1.25×, 1.5× and 2×. The current rate SHALL be visible. When pitch-kept audio is not
available at a rate, audio SHALL be muted at that rate and the interface SHALL say so.

#### Scenario: Change rate
- **WHEN** a person sets 1.5× and plays 10 seconds of timeline
- **THEN** playback finishes in about 6.7 seconds and the rate shows 1.5×

#### Scenario: Agent sets rate
- **WHEN** an agent sets the rate to 2× through the control link while the app is open
- **THEN** playback runs at 2× and the rate indicator shows 2×

### Requirement: Aspect-mismatch prompt
When a project's aspect ratio was never chosen, the first visual clip SHALL set it without a prompt. When the aspect was
chosen and no visual clip exists yet, adding a clip with a different aspect SHALL show an inline, non-blocking prompt to
keep or switch. Switching SHALL be one undoable revision. Imports from the CLI or MCP SHALL NOT prompt and SHALL return
a mismatch hint.

#### Scenario: Switch
- **WHEN** a person adds a 9:16 clip to a 16:9 template project with an empty timeline and chooses Switch
- **THEN** the project becomes 9:16 in one revision, and Undo returns it to 16:9

#### Scenario: Agent import
- **WHEN** an agent imports a 9:16 clip into a 16:9 project through MCP
- **THEN** the import succeeds, no prompt appears, and the result names the mismatch

### Requirement: Onboarding with the editor you know
First-run onboarding SHALL ask for the interface language and the editor the person used before. The answer SHALL set
the shortcut preset and layout and SHALL stay on this computer. Each step of the guided first edit SHALL offer "Show me",
which highlights the control to use.

#### Scenario: Preset from answer
- **WHEN** a new person chooses "Final Cut" as the editor they know
- **THEN** the Final Cut-style shortcut preset is active and no answer is sent anywhere

#### Scenario: Show me
- **WHEN** a person presses "Show me" on the "Add captions" step
- **THEN** the Captions panel opens and its button is highlighted

### Requirement: Local feedback with an optional screenshot
Help → Send feedback SHALL let the person include a screenshot of the app window. The screenshot SHALL be off by default,
SHALL blur the video frame by default, and SHALL be shown for review and cropping before saving. The report SHALL be
saved to a local folder or copied; nothing SHALL be sent automatically. An agent SHALL only create a draft that the
person reviews in the same dialog.

#### Scenario: Blurred by default
- **WHEN** a person turns the screenshot on and saves without changing other options
- **THEN** the saved image shows the panels clearly and the video frame blurred

#### Scenario: No network
- **WHEN** a person saves a feedback report
- **THEN** no network request is made

#### Scenario: Agent draft
- **WHEN** an agent reports a problem through MCP
- **THEN** a draft appears in the feedback dialog and nothing is saved or sent until the person acts

### Requirement: What's-new slides
After an update, the app SHALL show the release's slides once. A fresh install SHALL NOT show them. Slides SHALL come
from the installed app, not from the network, and a slide MAY open the feature it describes.

#### Scenario: Shown once
- **WHEN** a person updates and starts the app twice
- **THEN** the slides appear at the first start only

#### Scenario: Fresh install
- **WHEN** a person installs the app for the first time
- **THEN** no what's-new slides appear

### Requirement: Local activity and usage
Settings SHALL show a local view of revisions grouped by who made them (you, assistant, external agent, template,
generation) and provider spend per project and per chat, from data on this computer only.

#### Scenario: Grouped history
- **WHEN** a project has edits from the person and from an MCP agent
- **THEN** the view lists both groups with their counts and the agent's name

### Requirement: Light theme
Appearance SHALL offer Dark, Light and Same as system. The change SHALL apply at once.

#### Scenario: Follow system
- **WHEN** the theme is Same as system and the OS switches to light mode
- **THEN** the app switches to the light theme without a restart
