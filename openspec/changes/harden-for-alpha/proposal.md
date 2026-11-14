# Proposal: harden-for-alpha

## Why

Changes 1–16 have first implementations, verified on Linux only. An audit on 2026-11-12 found the code clean under
clippy and its tests, but not yet safe to hand to testers:

- A panic in a main-thread command can abort the app.
- A poisoned lock disables editing until restart.
- A failed export overwrite deletes the user's earlier file.
- API keys can reach log files at debug level.
- HDR phone footage looks washed out.
- Some UI copy and templates promise features that do not exist yet.
- CI checks only 9 of 17 packages.

This change closes those gaps before the first alpha.

## What Changes

- **Contained failures.** Every Tauri command, control-socket request, MCP message, and worker or generation thread
  runs inside panic containment and returns a plain-language error. Locks recover from poisoning. A dead playback
  engine restarts itself. Blocking I/O moves off the UI thread.
- **No silent loss.**
  - Export writes to a temporary file and renames it only on success, and refuses offline media.
  - Export reports when it falls back to a non-hardware, non-H.264 encoder.
  - Interchange export lists what will not carry over before writing.
  - The `captionComposition.fallback: hide` setting is honoured.
- **Secrets.**
  - Keys are redacted from all log output, and HTTP internals are capped at `warn`.
  - The fal key is sent only to fal's API hosts.
  - Release builds cannot override provider base URLs.
  - Ids are validated before becoming paths.
  - The Linux keyring is persistent.
- **Media correctness.**
  - PQ and HLG sources are tone-mapped to SDR in the compositor and the viewer.
  - Mixdown uses a peak limiter and a band-limited resampler.
  - Long transcriptions run in bounded windows.
  - Transcript cache keys are stable across Rust versions.
- **Honest UI.**
  - Every panel has a React error boundary.
  - No promise goes unhandled; type-aware lint enforces it.
  - Templates and buttons say what they actually do.
  - The assistant's seek tool moves the viewer.
- **Verification.**
  - Test fixtures are generated once, under a lock.
  - Tests that need media tools fail instead of passing vacuously.
  - CI lints and tests the whole workspace on macOS, Windows and Linux.
  - `deny.toml` scopes advisories to the shipped targets.

Out of scope (later changes): GPU compositor, proxies, subject masks, reframe tracking, loudness normalisation on
export, silence and filler removal, agent review/accept, signed installers. The R1 and R2 performance gates still
need the reference macOS and Windows machines.

References: docs/03-architecture-plan.md §5 (engineering standards); audit findings recorded in `design.md`.

## Capabilities

### New Capabilities
- `release-hardening`: failure containment, data-safety and secret-handling guarantees that apply across the app.

### Modified Capabilities
- None (requirements here are cross-cutting guarantees; feature specs are unchanged).

## Impact

Touches the desktop backend, ove-render export and audio, ove-media colour conversion, ove-gpu shaders, ove-agent,
ove-generate, ove-cli, ove-control, ove-transcribe, ove-interchange, the React UI, CI workflows and `deny.toml`.
New dependencies (all MIT/Apache): `parking_lot`, `rubato`, `tempfile` (runtime use), plus a small redaction module.
