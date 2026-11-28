# Spec Delta

## Purpose

Multicam editing: a group of synced camera and microphone members, a program item that selects what is seen and
heard, and angle switching that keeps sync.

## ADDED Requirements

### Requirement: Multicam groups
A multicam group SHALL be a timeline whose tracks are members. Each member SHALL have a role (angle, mic or both) and
a label. Member positions in the group SHALL define their sync. Creating a group from selected media SHALL sync the
members by audio and report the confidence per member.

#### Scenario: Create from three cameras
- **WHEN** the user creates a multicam group from three camera files that recorded the same audio
- **THEN** a group with three members is created, each placed at its synced offset, with a confidence per member

#### Scenario: Unrelated media
- **WHEN** one selected file shares no audio with the others
- **THEN** that file is left unsynced at time 0 and the report marks it with low confidence

### Requirement: Program item
A multicam program item SHALL show the selected angle member and play the selected audio members of a group, at the
item's time range and in point.

#### Scenario: Program plays the angle
- **WHEN** a program item selects angle cam_b and audio lav1
- **THEN** preview shows cam_b's picture and plays lav1's sound at the synced time

### Requirement: Angle switching
Switching angle over a range SHALL split the program item at the range edges and change the angle only inside the
range, as one revision, with sync kept. Switching to an angle with no media in the range SHALL be refused.

#### Scenario: Switch for 4 seconds
- **WHEN** the user switches to cam_c from 30 s to 34 s
- **THEN** the program shows cam_c from 30 s to 34 s and the previous angle before and after, in one revision

#### Scenario: No coverage
- **WHEN** cam_c stopped recording at 32 s and the user switches to cam_c from 30 s to 34 s
- **THEN** the switch is refused with code `multicam.noCoverage` and the uncovered times

### Requirement: Multicam surfaces
The viewer SHALL show a grid of angles for a multicam group and switch angle on click or number key. The command line
and MCP SHALL create groups, switch angles over ranges and list the program cut.

#### Scenario: Switch with a number key
- **WHEN** the user presses 2 during playback over a program item
- **THEN** the angle switches to member 2 at the playhead

#### Scenario: Agent reads the cut
- **WHEN** an agent asks for the program cut
- **THEN** it receives the ranges and the angle and audio members for each range
