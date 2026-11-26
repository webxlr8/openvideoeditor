# Proposal: add-workspace-modes-and-background-access

## Why

The editor has one fixed layout with four on/off panels and no splitters. There is no full-screen viewer and playback
has only 1× and 2× rates. External agents can reach the app only while a window is open: `ove app …` fails with exit
code 3 when the app is closed. A first clip with a different aspect ratio gives no prompt, so vertical phone footage
lands letterboxed. Feedback is a text report with no screenshot. Release notes are a static page that nobody sees after
an update. The app has only a dark theme.

## What Changes

- Workspace layout presets (Edit, Assistant, Review, Vertical) with draggable splitters, sizes saved per preset, and a
  key to maximise the focused panel. A 9:16 project suggests the Vertical preset once.
- A full-screen viewer with floating controls (scrub, timecode, loop, playback rate, safe-area guides) and playback
  rates 0.5×, 1×, 1.25×, 1.5× and 2×. Agents can set the rate through the control link.
- Background mode: an optional "Keep running when the window is closed" setting, off by default. A tray or menu-bar
  icon shows which project an agent is editing and gives Open, Recent projects, Pause agent access and Quit. Agents can
  open a project with no window. `ove app start --background` starts the app in this mode.
- OS notifications when an export or a generation finishes.
- A non-blocking aspect-mismatch prompt when the first clip in a project has a different aspect ratio. CLI and MCP
  imports never block and return a hint.
- Onboarding v2: language choice, "the editor you know" (sets the shortcut preset and layout, kept on this computer
  only), and "Show me" spotlights for each step of the guided first edit.
- Feedback with an optional window screenshot: off by default, the video frame blurred by default, crop before saving,
  saved to a local folder. Agents can only write a draft that the person reviews.
- Bundled what's-new slides shown once after an update, with "Try it" links to the feature. No network fetch.
- Settings → Storage hand-off: this change adds the pane frame and links; the cache, proxy and model entries come from
  add-background-tasks-and-media-derivatives and add-model-runtime.
- A local Activity and Usage view: revisions grouped by who made them, and provider spend from the local cost ledger.
- A light theme and "Same as system", built on the theme tokens of add-preferences-i18n-and-keymaps.

Out of scope: launch at login, a second-display video output, signed in-place updates (add-distribution), sending
anything off the computer, accounts, telemetry, remote what's-new content, layout editing beyond the presets.

References: docs/spec/desktop-shell.md (windowing, menus, View › fullscreen, Help, updates),
docs/spec/presets.md (Keyboard shortcuts: Toggle Fullscreen, Toggle Chat Panel, Toggle Left Panel, Toggle Timeline).

## Capabilities

### New Capabilities
- `workspace-modes`: layout presets, splitters, maximise, full-screen viewer, playback rates, aspect-mismatch prompt,
  onboarding v2, feedback with screenshot, what's-new slides, activity and usage view, light theme.
- `background-access`: background mode, tray menu, headless project open for agents, pause of agent access, and
  finish notifications.

### Modified Capabilities
- None.

## Impact

- UI: `Editor.tsx` layout, a new splitter component, viewer full-screen mode, `onboarding/`, `Support.tsx`, Settings
  panes, what's-new content folder.
- Native shell: tray icon, window close behaviour, activation policy on macOS, control-link `open` with
  `background: true`, transport rate command, notifications, webview snapshot for feedback.
- CLI: `ove app start --background`; import results gain an aspect hint.
- Depends on add-preferences-i18n-and-keymaps (preferences store, native strings, theme tokens, keymap presets).
  Uses the actor on revisions from unify-agent-tool-layer for the Activity view, and the storage entries from
  add-background-tasks-and-media-derivatives and add-model-runtime.
