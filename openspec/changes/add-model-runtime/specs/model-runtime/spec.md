# Spec Delta

## Purpose

Local machine-learning models: how they are installed, checked, listed and removed, when the person must consent,
and how inference runs on macOS and Windows within the product's memory budget.

## ADDED Requirements

### Requirement: Verified installation
Every model file SHALL be downloaded from a fixed, versioned location and SHALL be accepted only when its size and
SHA-256 match the pinned values. A file SHALL become visible to features only after it is complete and verified.

#### Scenario: Good download
- **WHEN** a model is installed and every file matches its pinned size and hash
- **THEN** the model is reported as installed and features can use it

#### Scenario: Corrupt download
- **WHEN** a downloaded file does not match its pinned hash
- **THEN** the file is deleted, the model stays not installed, and the error names the file and says the download was
  corrupt

#### Scenario: Unchanged after reinstall
- **WHEN** an installed model is installed again
- **THEN** no file is downloaded and the result says it is already installed

### Requirement: Resumable download with space check
A download SHALL check free disk space before it starts and SHALL continue from the bytes already received after an
interruption. A cancelled download SHALL leave no partial model visible to features.

#### Scenario: Not enough space
- **WHEN** free disk space is less than the model size plus 10 %
- **THEN** the download does not start and the error states the space needed and the space free

#### Scenario: Resume after interruption
- **WHEN** a download is interrupted at 60 % and started again
- **THEN** it continues from the received bytes and the finished file passes verification

#### Scenario: Cancel
- **WHEN** the person cancels a download
- **THEN** it stops within 1 second and the model is still reported as not installed

### Requirement: Consent for large models
A model or runtime larger than 50 MB SHALL download only after the person confirms in the app, with the size shown.
A request from an agent, the CLI or MCP SHALL NOT start such a download; it SHALL return a "not installed" result with
the model name, size and how to install it. A model of 50 MB or less MAY download when a feature first needs it, with
visible progress.

#### Scenario: Agent asks for a large model
- **WHEN** an agent calls a feature that needs a 450 MB model that is not installed
- **THEN** the call returns a "model not installed" result with the size, and nothing is downloaded

#### Scenario: Person confirms
- **WHEN** the person confirms the download prompt in the app
- **THEN** the download starts with progress and can be cancelled

#### Scenario: Small model
- **WHEN** a feature first needs a 2 MB model
- **THEN** it downloads with visible progress and the feature continues when it is ready

### Requirement: Inference on both platforms
Every model a feature uses SHALL run on macOS (Apple silicon) and Windows 11 (x64) with a CPU path that needs no GPU.
For the same input, CPU results on both platforms SHALL agree within the tolerance the feature's change states.

#### Scenario: No GPU
- **WHEN** a model runs on a machine where no hardware acceleration is available
- **THEN** it runs on the CPU and produces results within tolerance

#### Scenario: Runtime missing
- **WHEN** a feature needs inference and the runtime library is not installed
- **THEN** the feature reports "runtime not installed" with the size and install action, and does not crash

### Requirement: Memory budget
Running a model larger than 10 MB SHALL NOT raise the editor process's resident memory by more than 50 MB. Such models
SHALL run outside the editor process, and that process SHALL exit after 2 minutes without work.

#### Scenario: Heavy model idle
- **WHEN** a large model finished its last job 2 minutes ago
- **THEN** the inference process has exited and its memory is released

#### Scenario: Worker crash
- **WHEN** the inference process crashes during a job
- **THEN** the job fails with an error, the editor keeps running, and the next job starts a new process

### Requirement: Storage controls
The person SHALL be able to see every installed model with its name, purpose, size and location, and remove it. The
same list SHALL be available from the CLI and MCP.

#### Scenario: Remove a model
- **WHEN** the person removes an installed model in Settings → Storage
- **THEN** its files are deleted, the free space is shown, and features that need it report it as not installed

#### Scenario: Remove while in use
- **WHEN** the person removes a model that a running job uses
- **THEN** the removal waits until the job ends or is cancelled, and says so

### Requirement: Whisper models keep working
Whisper models installed before this change SHALL keep working without a second download.

#### Scenario: Upgrade
- **WHEN** the app starts after an upgrade with a whisper model already on disk
- **THEN** the model is listed as installed and captions run without downloading it again
