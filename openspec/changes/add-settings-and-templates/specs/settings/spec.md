# Spec Delta

## Purpose

People need one place for keys, shortcuts and privacy, and starting points for common videos.

## ADDED Requirements

### Requirement: Keys never in files
Provider and assistant keys SHALL be kept in the system credential store and never written to project files or logs.

#### Scenario: Keys never in files
- **WHEN** a key is saved in Settings
- **THEN** no project file or log contains it

### Requirement: Rebinding
Single-key editor actions SHALL be rebindable; binding a key used elsewhere SHALL move it, and resetting SHALL restore the defaults.

#### Scenario: Rebinding
- **WHEN** Split is moved to X
- **THEN** X splits, S does nothing, and Reset restores S
