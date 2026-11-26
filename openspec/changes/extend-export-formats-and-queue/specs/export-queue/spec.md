# Spec Delta

## Purpose

Export jobs that wait in a queue, survive a restart, render a fixed revision, and can be watched and cancelled from the
app, the command line and agents.

## ADDED Requirements

### Requirement: Persistent export queue
Exports SHALL be jobs in one queue shared by all projects, stored on disk. A job SHALL record its project, the revision
it renders, its settings, output path, status, progress, the encoders used, warnings, errors and who requested it
(person, assistant, command line or the agent client's name). The app, the command line and MCP SHALL list the same
jobs.

#### Scenario: Same queue everywhere
- **WHEN** a person queues an export in the app and an agent lists export jobs over MCP
- **THEN** the agent sees the job with the same id, status and progress

#### Scenario: Requester recorded
- **WHEN** an MCP client named "claude-code" queues an export
- **THEN** the job shows that client name as the requester in the Exports panel

### Requirement: Revision pinning
A queued export SHALL render the project exactly as it was at the revision current when the job was queued. Later
edits SHALL NOT change the output. If that revision is no longer available when the job starts, the job SHALL fail
with a message.

#### Scenario: Edit while waiting
- **WHEN** a person queues an export, then deletes a clip before the job starts
- **THEN** the exported file still contains the deleted clip

#### Scenario: Revision unavailable
- **WHEN** a job starts and its pinned revision cannot be reconstructed
- **THEN** the job fails with a message to export again, and no output file is written

### Requirement: Background execution and progress
Queued exports SHALL run in a background process that keeps running when the app window closes. Jobs SHALL start in
queue order. Progress SHALL update at least once per second while a job runs. The output SHALL appear at its final
path only when the job succeeds.

#### Scenario: Several jobs in order
- **WHEN** three exports are queued
- **THEN** they run in the order they were queued, and each output file appears only when its job is done

#### Scenario: Failure leaves no file
- **WHEN** a job fails during encoding
- **THEN** no file exists at the output path and the job shows the error

### Requirement: Cancel
A queued or running export SHALL be cancellable from the app, the command line and MCP. Cancelling a running job SHALL
stop it within 2 seconds and delete its partial output.

#### Scenario: Cancel running job
- **WHEN** an agent cancels a running export
- **THEN** the job status becomes cancelled within 2 seconds and no partial file remains

#### Scenario: Cancel finished job
- **WHEN** a person cancels a job that has already finished
- **THEN** the request is refused with a message, and the output file is kept

### Requirement: Restart recovery
After the app or the computer restarts, queued jobs SHALL still be in the queue, and a job that was running SHALL
restart from the beginning. A job that fails to finish after 3 attempts SHALL be marked failed.

#### Scenario: Quit during export
- **WHEN** the app quits while one job runs and two wait, and the app is opened again
- **THEN** all three jobs finish, and the interrupted one restarted from the beginning

#### Scenario: Repeated crash
- **WHEN** a job's worker process ends abnormally three times
- **THEN** the job is marked failed with a message, and the next queued job starts

### Requirement: Queue surfaces
The command line SHALL list, watch and cancel export jobs and SHALL be able to queue a job and return at once. MCP
`export_video` SHALL honour format, quality, range and wait, and `job_status` and `cancel_job` SHALL accept export job
ids. The app SHALL show an Exports panel with each job's progress and actions to show the file, cancel and export again.

#### Scenario: Detached export from the command line
- **WHEN** `ove export <project> out.mov --preset "ProRes 422 HQ master" --detach --json` runs
- **THEN** it returns a job id at once with exit code 0, and `ove export status <id> --wait` returns when the job is done

#### Scenario: Unknown job id
- **WHEN** `job_status` is called with an id that does not exist
- **THEN** the tool returns an error that names the id
