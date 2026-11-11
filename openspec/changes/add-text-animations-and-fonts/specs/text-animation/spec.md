# Spec Delta

## Purpose

Text needs entrances and exits and real typefaces to look finished.

## ADDED Requirements

### Requirement: Animations settle exactly
Every text animation SHALL change the text during its phase and leave it exactly at rest afterwards.

#### Scenario: Animations settle exactly
- **WHEN** a title has the Rise animation
- **THEN** frames after the animation equal the frame without it
