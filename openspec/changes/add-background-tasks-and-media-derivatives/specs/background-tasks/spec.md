# Spec Delta

## Purpose

Background work in the app and the CLI: how jobs are ordered, limited, paused, cancelled and shown, so that editing and
playback stay smooth while derived data is built.

## ADDED Requirements

### Requirement: Priorities
Background work SHALL run in priority order: export first, then interactive requests (for example a thumbnail the
person is looking at), then media derivatives, then analysis and indexing. A waiting task SHALL gain priority with age
so that low-priority work is never starved for more than 10 minutes while the app is idle.

#### Scenario: Export goes first
- **WHEN** indexing tasks are running and the person starts an export
- **THEN** the export starts within 1 second and the indexing tasks yield their resources

#### Scenario: No starvation
- **WHEN** interactive requests arrive continuously for 10 minutes while one analysis task waits
- **THEN** the analysis task has started by the end of that period

### Requirement: Resource limits and memory budget
Each task SHALL declare its resource class and estimated peak memory. The scheduler SHALL NOT run more tasks of a class
than that class allows, and SHALL NOT start a task whose estimate would take the total over the background memory
budget (the smaller of 1.5 GB and 20 % of physical memory). Background threads SHALL run at the operating system's
background priority.

#### Scenario: Budget respected
- **WHEN** queued tasks together estimate more than the budget
- **THEN** only tasks that fit start, and the rest wait until memory is released

#### Scenario: Task larger than budget
- **WHEN** a single task estimates more than the whole budget
- **THEN** it runs alone when nothing else is running, and the popover says it is running alone

### Requirement: Automatic pause
Derivative and analysis tasks SHALL pause while the timeline is playing, while an export runs, while the computer is on
battery power or in low-power mode, and under thermal pressure, and SHALL resume when the condition ends. The person
SHALL be able to pause and resume all background work.

#### Scenario: Playback pauses work
- **WHEN** the person starts playback while skim sheets are being built
- **THEN** those tasks pause within 200 ms and playback drops no frames because of them

#### Scenario: On battery
- **WHEN** the laptop switches to battery power
- **THEN** analysis tasks pause and the popover shows "Paused: on battery", with a button to run anyway

### Requirement: Cancellation
Every task SHALL be cancellable. A cancelled task SHALL stop within 200 ms (a task running in another process within
1 second) and SHALL leave no partial output visible to features.

#### Scenario: Cancel one task
- **WHEN** the person cancels a running proxy task
- **THEN** it stops, its partial file is removed, and other tasks continue

#### Scenario: Cancel by agent
- **WHEN** an agent cancels a task by id through MCP
- **THEN** the task stops and the result reports it as cancelled

### Requirement: Tasks are idempotent and re-derived
Background results SHALL be keyed by their inputs so that running a task twice gives the same result and a finished
result is reused. The queue SHALL NOT be saved across restarts; opening a project SHALL re-scan and queue only missing
results.

#### Scenario: Reopen after quit
- **WHEN** the app quits while skim sheets are half built and the project is opened again
- **THEN** finished sheets are reused and only the missing ones are queued

### Requirement: Visibility
The app SHALL show running, waiting and paused tasks with name, project, progress and pause reason in a Background
Tasks popover. The CLI and MCP SHALL list tasks, cancel them and wait for them to finish. Progress SHALL be reported at
most 4 times per second per task.

#### Scenario: Agent waits for indexing
- **WHEN** an agent calls wait-for-tasks with a project and a kind
- **THEN** the call returns when those tasks are done, or at the time-out with the remaining count

#### Scenario: Nothing running
- **WHEN** no background work exists
- **THEN** the status bar shows no task indicator and the app's idle memory stays within its budget

### Requirement: Existing jobs on the scheduler
Captions, export and import thumbnails SHALL run as scheduled tasks with their existing results and errors. Import
SHALL return to the person before thumbnails and probes are finished, and SHALL show them as they complete.

#### Scenario: Large import
- **WHEN** the person imports 200 files
- **THEN** the media panel lists them at once with placeholders, and thumbnails appear as tasks finish

#### Scenario: Caption job failure
- **WHEN** a caption task fails because its model is missing
- **THEN** the same error as before is shown and other tasks continue
