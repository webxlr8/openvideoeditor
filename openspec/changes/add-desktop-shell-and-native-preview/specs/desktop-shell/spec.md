# Spec Delta

## Purpose

The desktop application frame: its window, single-instance behaviour, routing of opened files, the bridge between the
native core and the UI, and the native viewer surface embedded in the UI.

## ADDED Requirements

### Requirement: Application window
The app SHALL open one main window at launch on macOS 14+ (Apple silicon) and Windows 11 x64, in a dark appearance,
restoring the last window size, position and display when that display is still connected, and otherwise centring on
the primary display at a default size of 1440×900 logical pixels (clamped to the display's work area). The window
SHALL have a minimum size of 1024×640 logical pixels.

#### Scenario: First launch
- **WHEN** the app is launched for the first time
- **THEN** a window of 1440×900 logical pixels (or the work area, if smaller) appears centred on the primary display

#### Scenario: Restore on a disconnected display
- **WHEN** the app was last closed on a display that is no longer connected
- **THEN** the window opens fully visible on the primary display at its last size, clamped to the work area

#### Scenario: Cold start budget
- **WHEN** the app is cold-started on the reference machines (M1 MacBook Air, 8 GB; Windows 11 laptop, 4-core, 16 GB)
- **THEN** the UI is interactive within 1.5 s of process start

### Requirement: Single instance and open-file routing
The app SHALL run as a single instance. Launching it again, opening a supported media file with it, or dropping a file
on its Dock/taskbar icon SHALL focus the running window and route the file paths to it.

#### Scenario: Second launch
- **WHEN** the app is running and the user launches it again
- **THEN** no second process stays running and the existing window is focused

#### Scenario: Open a file with the app while it is running
- **WHEN** the user chooses "Open with" on a supported video file while the app is running
- **THEN** the running window receives the path and opens the file in the viewer

#### Scenario: Unsupported file
- **WHEN** the app receives a path it cannot open
- **THEN** it shows a plain-language message naming the file and the reason, and the current state is unchanged

### Requirement: Typed UI bridge
All communication between the native core and the UI SHALL go through one typed command/event bridge whose types are
generated from the native definitions. Every command SHALL return either a result or a structured error with a code
and a plain-language message; a failure in a command SHALL NOT crash the app.

#### Scenario: Command error
- **WHEN** a UI command fails inside the native core (for example an unreadable file)
- **THEN** the UI receives a structured error with a code and message, the app keeps running, and the error is logged

#### Scenario: Contract drift
- **WHEN** a native command or event type changes and the UI bindings were not regenerated
- **THEN** the build fails

### Requirement: Native viewer surface placement
The viewer SHALL be drawn by a native GPU surface aligned exactly with the viewer rectangle laid out by the UI, and
SHALL never draw outside that rectangle. Hiding the viewer area in the UI SHALL hide the surface.

#### Scenario: Viewer hidden
- **WHEN** the UI collapses the viewer area
- **THEN** no video pixels are drawn anywhere in the window

### Requirement: Viewer follows layout changes
The viewer surface SHALL follow the viewer rectangle within one displayed frame on window resize, move, entering or
leaving fullscreen, moving between displays, scale changes and panel layout changes.

#### Scenario: Resize
- **WHEN** the user drags the window edge continuously
- **THEN** the viewer image stays aligned with the viewer rectangle with no visible gap or overdraw larger than one
  physical pixel after each frame settles

#### Scenario: Move between displays with different scale
- **WHEN** the window moves from a 2× display to a 1× display (or 150% to 100% on Windows)
- **THEN** the viewer re-renders sharp at the new scale and stays aligned

### Requirement: UI layers above the viewer
Menus, dialogs, tooltips and drag previews from the UI SHALL appear above the viewer and remain fully usable where
they overlap it.

#### Scenario: Menu over the viewer
- **WHEN** a UI dropdown or dialog opens over the viewer area
- **THEN** it is fully visible and clickable above the video

### Requirement: GPU device loss recovery
The viewer SHALL recover from GPU device loss without restarting the app.

#### Scenario: GPU device loss
- **WHEN** the GPU device is lost (driver reset or display sleep on Windows, GPU restart on macOS)
- **THEN** the viewer recreates its surface and resumes showing the current frame within 2 s without restarting the app

### Requirement: Crash containment and logs
The app SHALL write rotating logs to the platform log location and SHALL record the decode path, GPU adapter and OS
version at startup. A panic in a background task SHALL be caught, logged and reported to the UI without terminating
the app.

#### Scenario: Background task panic
- **WHEN** a decode worker panics
- **THEN** playback stops with an error message, the log contains the panic and backtrace, and the window stays usable
