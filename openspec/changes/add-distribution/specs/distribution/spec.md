# Spec Delta

## Purpose

People need installers, a way to learn about new versions, and help when something breaks — without the app phoning home.

## ADDED Requirements

### Requirement: No data leaves without consent
The app SHALL NOT send usage data; crash reports SHALL stay on the computer unless the person copies one into feedback.

#### Scenario: No data leaves without consent
- **WHEN** the app panics
- **THEN** a report is written next to the logs and nothing is sent
