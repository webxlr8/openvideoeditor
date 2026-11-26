# Spec Delta

## Purpose

Keyboard shortcuts as one registry: the 27 reference actions, 5 presets, rebinding with conflict checks, keymap files,
menu accelerators generated from the same source, and letter keys that work on any keyboard layout. Also theme tokens
and per-kind clip colours.

## ADDED Requirements

### Requirement: Action registry
The app SHALL provide the 27 actions listed in docs/spec/presets.md (Keyboard shortcuts) with their default chords.
"Command" in the reference SHALL mean Command on macOS and Ctrl on Windows. Every action SHALL be rebindable.

#### Scenario: Default chord
- **WHEN** a person presses the Split at Playhead default chord with a clip under the playhead
- **THEN** the clip splits at the playhead

#### Scenario: Platform modifier
- **WHEN** a person on Windows presses Ctrl+Z after an edit
- **THEN** the edit is undone, as Command+Z does on macOS

### Requirement: Presets
Settings → Shortcuts SHALL offer the presets Default, Premiere-style, Resolve-style, Final Cut-style and CapCut-style.
Each preset SHALL apply the overrides listed in the reference spec and keep Default for all other actions. Choosing a
preset SHALL keep the person's own rebinds unless they choose "Reset to preset".

#### Scenario: Apply preset
- **WHEN** a person chooses Resolve-style
- **THEN** N toggles snapping and P toggles fullscreen

#### Scenario: Own rebind kept
- **WHEN** a person has rebound Export and then chooses another preset
- **THEN** Export keeps the person's chord and the other actions follow the preset

### Requirement: Rebinding with conflict checks
A person SHALL rebind an action by pressing a new chord. When the chord is already used, the app SHALL name the other
action and offer to swap or cancel. Chords that the OS reserves SHALL be refused with a reason.

#### Scenario: Conflict
- **WHEN** a person binds Mute Selected to the chord used by Toggle Snapping
- **THEN** the app names Toggle Snapping and offers Swap or Cancel, and nothing changes until the person chooses

#### Scenario: Reserved chord
- **WHEN** a person on macOS tries to bind Command+H
- **THEN** the app refuses and says the OS uses that chord

### Requirement: Keymap files
A person SHALL export the current keymap to a file and import a keymap file. Import SHALL refuse unknown file versions
and report unknown action ids without applying them.

#### Scenario: Round trip
- **WHEN** a person exports a keymap on macOS and imports it on Windows
- **THEN** every action has the same chord, with Command mapped to Ctrl

#### Scenario: Invalid file
- **WHEN** a person imports a file that is not a keymap
- **THEN** the app refuses with a message and the current keymap is unchanged

### Requirement: One source for menus and overlay
Native menu accelerators and the shortcut overlay SHALL show the chords of the current keymap. A rebind SHALL update
both at once.

#### Scenario: Menu follows rebind
- **WHEN** a person rebinds Export to Command+Shift+X
- **THEN** the File menu shows Command+Shift+X next to Export and the overlay lists the same chord

### Requirement: Layout-independent letter keys
Letter shortcuts SHALL work when the active keyboard layout does not produce Latin letters, by using the physical key
position.

#### Scenario: Cyrillic layout
- **WHEN** a person types with a Russian layout and presses the physical key for Toggle Snapping
- **THEN** snapping toggles

### Requirement: Clip colours per kind
Settings → Appearance SHALL let a person set one colour per clip kind (video, audio, image, text, caption, motion,
generated), choose a colour-blind-safe preset, and reset to defaults. Clip label text SHALL stay readable on every
chosen colour.

#### Scenario: Change audio colour
- **WHEN** a person sets the audio clip colour to green
- **THEN** every audio clip on the timeline turns green at once and keeps that colour after restart

#### Scenario: Readable label
- **WHEN** a person picks a very light clip colour
- **THEN** clip labels switch to dark text that meets the WCAG AA contrast ratio
