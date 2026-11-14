# Spec Delta

## Purpose

A person who has never seen the app gets from the first launch to a first exported video without reading anything.

## ADDED Requirements

### Requirement: First run
On the first launch the app SHALL show a welcome of at most three screens (what the app does, pick an assistant,
privacy), each of which can be skipped, and SHALL NOT show it again once finished or skipped.

#### Scenario: First run
- **WHEN** the app starts with no onboarding state
- **THEN** the welcome is shown before Home

#### Scenario: Skipped
- **WHEN** Skip is pressed on any screen
- **THEN** Home is shown and the welcome does not return at the next launch

### Requirement: The welcome steps aside for a project opened from outside
When an agent or the command line opens a project in the app while the welcome is showing, the welcome SHALL close
for that run without being finished or skipped.

#### Scenario: Opened from outside
- **WHEN** `ove app open` opens a project during a first run
- **THEN** the editor shows that project, and the welcome is shown again at the next launch

### Requirement: Onboarding state is the app's
Onboarding state SHALL be kept in the app's configuration folder by the native side, not in the web profile.

#### Scenario: Onboarding state is the app's
- **WHEN** the web profile is cleared after the welcome was finished
- **THEN** the welcome is not shown again

### Requirement: Pick an assistant
The welcome SHALL show the supported agents with their icons and state, and SHALL accept "I'll edit by hand". It
SHALL NOT ask for a key.

#### Scenario: Pick an assistant
- **WHEN** two agents are Ready
- **THEN** both can be chosen, and the choice is the chat panel's assistant afterwards

### Requirement: Sample project
The app SHALL offer a sample project made from media shipped with it, holding a cut, a title, captions and a
transition. Opening it SHALL copy it into the library and SHALL leave the shipped files unchanged.

#### Scenario: Sample project
- **WHEN** "Try the sample" is chosen
- **THEN** the editor opens a project with clips on the timeline and a picture in the viewer

#### Scenario: Sample twice
- **WHEN** the sample is opened a second time
- **THEN** a second copy is made and the first keeps its edits

### Requirement: Guided first edit
The app SHALL offer a checklist of Import, Cut, Title, Captions and Export. A step SHALL be ticked when the project
shows it was done, and the list SHALL be closable and reopenable from Help.

#### Scenario: A step is done
- **WHEN** the person splits a clip
- **THEN** Cut is ticked without any further action

#### Scenario: Reopen
- **WHEN** the checklist was closed and Help → Getting started is chosen
- **THEN** the checklist returns with the same steps ticked

### Requirement: Hints in empty panels
Every empty panel SHALL say what it is for in one sentence and offer one action.

#### Scenario: Hints in empty panels
- **WHEN** the Captions panel is opened in a project without captions
- **THEN** it says what captions do and offers to make them

### Requirement: Starter prompts
An empty chat SHALL offer three one-tap example requests and an answer to "What can you do?".

#### Scenario: Starter prompts
- **WHEN** a starter prompt is tapped
- **THEN** it is placed in the message box, not sent

### Requirement: Help
Home and the editor SHALL have one Help entry that reopens the welcome, the checklist and the sample, and lists the
keyboard shortcuts.

#### Scenario: Help
- **WHEN** Help is opened
- **THEN** Getting started, the sample project and the shortcuts can each be reached in one step
