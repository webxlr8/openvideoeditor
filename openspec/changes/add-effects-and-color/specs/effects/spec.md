# Spec Delta

## Purpose

Looks, transitions and layouts turn cuts into finished videos.

## ADDED Requirements

### Requirement: Safe parameters
Effects SHALL clamp parameters to their contracts so any input renders without NaN or panic.

#### Scenario: Safe parameters
- **WHEN** an effect gets values below min and above max
- **THEN** it renders using the clamped values
