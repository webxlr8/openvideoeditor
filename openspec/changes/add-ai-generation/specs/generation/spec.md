# Spec Delta

## Purpose

Missing shots, voice-over and music are the last gaps between a rough cut and a finished video.

## ADDED Requirements

### Requirement: Nothing spent without confirmation
A generation SHALL show its estimated cost and start only after explicit confirmation (`--yes`, a second click).

#### Scenario: Nothing spent without confirmation
- **WHEN** the person clicks Generate once
- **THEN** no request is sent and the button asks to confirm the amount

### Requirement: Placeholders filled in place
Starting a job SHALL place a pending placeholder; finishing SHALL register the output in the bundle and fill that placeholder keeping its id.

#### Scenario: Placeholders filled in place
- **WHEN** an image job finishes after the placeholder was moved
- **THEN** the image appears where the placeholder was moved to
