# Spec Delta

## Purpose

Captions are expected on almost every social video and must work offline.

## ADDED Requirements

### Requirement: Offline transcription
Transcription SHALL run on this computer with a downloaded model and never send audio elsewhere.

#### Scenario: Offline transcription
- **WHEN** the computer is offline and the model is installed
- **THEN** auto-captions completes and the captions track is filled

### Requirement: One revision per caption run
Auto-captions SHALL add or replace captions as a single revision aligned to the timeline through clip trim, speed and direction.

#### Scenario: One revision per caption run
- **WHEN** a clip is trimmed by 2 s and played at 2×
- **THEN** caption times match the spoken words on the timeline within one frame
