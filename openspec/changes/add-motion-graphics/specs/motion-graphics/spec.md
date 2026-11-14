# Spec Delta

## Purpose

Titles, lower thirds and animated graphics are the most requested additions after cuts. Agents need to write them as code they can reason about, and people need templates they can fill in.

## ADDED Requirements

### Requirement: Sandboxed, deterministic code items
Code items SHALL render in a sandbox with no file system, network, timers or module loading, within memory and per-frame CPU limits, and the same composition at the same time SHALL produce identical pixels.

#### Scenario: Sandboxed, deterministic code items
- **WHEN** a composition reads `Date.now()` and `Math.random()`
- **THEN** two renders of the same frame are byte-identical

### Requirement: Checked before use
`check` SHALL report syntax errors, unavailable globals, runtime errors and limit breaches with line and column, and render the start, middle and end frames.

#### Scenario: Checked before use
- **WHEN** a composition loops forever
- **THEN** check reports the CPU limit at the offending frame and the item renders nothing instead of hanging
