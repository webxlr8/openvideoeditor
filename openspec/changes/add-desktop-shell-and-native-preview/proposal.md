# Proposal: add-desktop-shell-and-native-preview

## Why

The whole architecture rests on one unproven assumption: a Tauri window with a system web view UI can host a native
GPU viewer that plays 4K camera media smoothly, with hardware decode, on both macOS and Windows 11 (risk R1 in
docs/03-architecture-plan.md). If this fails, the UI approach changes, so it must be proven first, before any other UI
work. The shell built here is kept as the product's foundation, not thrown away.

## What Changes

- New Rust workspace and Tauri 2 app that opens one window (dark, hidden-inset title bar on macOS, standard frame on
  Windows) with a React UI placeholder for Home and an editor-like layout containing a viewer area.
- Native viewer surface rendered with the GPU (Metal on macOS, Direct3D 12 on Windows) placed exactly over the viewer
  rectangle of the web UI and following it on resize, move, fullscreen, display change and DPI change.
- Single-file playback in the viewer: open a local video file, play/pause, frame-accurate seek and step, scrub,
  looping, with hardware decode first and automatic software fallback.
- Audio output with audio-master A/V sync, volume and mute.
- Typed command/event bridge between Rust and the UI, generated from Rust types.
- Diagnostics overlay and a headless benchmark command that record dropped frames, seek latency, decode path and
  memory, so the gate is decided by numbers.
- Dev tooling: CI on macOS arm64 and Windows 11 x64, licence/advisory checks, LGPL-only FFmpeg build scripts.

Out of scope: projects, timeline, multiple clips, compositing, effects, export, tabs beyond one window, menus beyond a
minimal app menu, signing/updates (change 16), the frame-streaming fallback (built only if the gate fails).

References: docs/spec/desktop-shell.md (window chrome, single instance, open-file routing); docs/01-reference-teardown.md
(viewer behaviour).

## Capabilities

### New Capabilities
- `desktop-shell`: application window, single-instance behaviour, open-file routing, UI bridge, native viewer surface
  placement and lifecycle.
- `media-playback`: opening a media file for preview, decode path selection, playback transport, seeking, audio
  output and A/V sync, playback diagnostics.

### Modified Capabilities
- None.

## Impact

- New repository layout: `crates/` (Rust), `apps/desktop/` (Tauri + React), `tools/ffmpeg/` (build scripts), CI
  workflows.
- New dependencies: Tauri 2.12.x, wgpu 30.x, ffmpeg-the-third 6.x with FFmpeg 9.x (LGPL build, dynamic), cpal,
  React 19, Vite, TypeScript, cargo-deny.
- Bundled FFmpeg shared libraries become part of the app; their size counts against the 60 MB installer budget.
