# Spec Delta

## Purpose

Letting agents reach the app when no window is open, while the person can always see and stop that access: background
mode, the tray menu, headless project open, pause of agent access, and finish notifications.

## ADDED Requirements

### Requirement: Background mode
Settings → General SHALL offer "Keep running when the window is closed", off by default. When it is on, closing the last
window SHALL keep the app running with a tray or menu-bar icon. When it is off, closing the last window SHALL quit as
today.

#### Scenario: Off by default
- **WHEN** a person closes the last window with default settings
- **THEN** the app quits

#### Scenario: Keep running
- **WHEN** background mode is on and the person closes the last window
- **THEN** the app keeps running, a tray icon is visible, and `ove app context` succeeds

#### Scenario: First close notice
- **WHEN** the window is closed in background mode for the first time
- **THEN** one OS notification says the app is still running and how to quit

### Requirement: Tray menu
The tray menu SHALL offer Open, Recent projects, the name of the project an agent is editing, Pause agent access and
Quit.

#### Scenario: Agent activity shown
- **WHEN** an MCP agent applies an edit to a project with no window open
- **THEN** the tray menu shows that project as being edited by an agent

#### Scenario: Quit
- **WHEN** the person chooses Quit in the tray menu
- **THEN** the app saves its state and exits, and later `ove app` commands return the "app not running" exit code

### Requirement: Headless project open
An agent SHALL open a project through the control link without opening a window. Edits SHALL appear as revisions, and
opening the project's window later SHALL show them. `ove app start --background` SHALL start the app in background mode
when it is not running.

#### Scenario: Edit without window
- **WHEN** an agent opens a project in the background, applies an edit, and the person then opens the project
- **THEN** the editor shows the edit and the History lists it

#### Scenario: Lock held
- **WHEN** an agent asks to open in the background a project that another process holds
- **THEN** the request fails with the lock-held error and names the holder

### Requirement: Pause agent access
The person SHALL pause agent access from the tray or the editor. While paused, the control link SHALL refuse write
requests with a clear error, and reads MAY continue. Resume SHALL restore access.

#### Scenario: Paused
- **WHEN** agent access is paused and an agent tries to apply an edit
- **THEN** the edit is refused with an "agent access paused" error and the document is unchanged

### Requirement: Finish notifications
The app SHALL send an OS notification when an export or a generation finishes or fails, unless the person turned
notifications off.

#### Scenario: Export finished in background
- **WHEN** an export finishes while no window is open
- **THEN** an OS notification names the file, and clicking it opens the containing folder

#### Scenario: Notifications off
- **WHEN** notifications are off and a generation fails
- **THEN** no OS notification appears and the failure is listed in the project
