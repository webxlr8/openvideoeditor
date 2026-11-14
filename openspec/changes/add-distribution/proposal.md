# Proposal: add-distribution

## Why

People need installers, a way to learn about new versions, and help when something breaks — without the app phoning home.

## What Changes

- Installers per platform with the LGPL FFmpeg libraries and the `ove` CLI as a sidecar (tools/package.sh, release workflow).
- Crash reports written locally by the panic hook and surfaced on the next launch; no telemetry.
- Update check against a manifest URL configured at build time.
