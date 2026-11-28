# Design: add-workspace-modes-and-background-access

## Context

See proposal.md (Why). `Editor.tsx` holds a `Layout` of four booleans and fixed panel sizes. The native viewer sits
under a transparent web view (macOS) or streams frames into a canvas (Windows), and `viewerRect.ts` reports its rect.
`transport.rs` supports shuttle at ±1× and ±2× only. Closing the window ends the app (inferred from `CloseRequested`
handling in `lib.rs`). The control link is loopback with a per-launch token. Onboarding is a 3-screen welcome plus a
checklist that ticks from project state. `Support.tsx` builds a text report. `releaseNotes.ts` is static.
Requirements: specs/workspace-modes, specs/background-access.

## Goals / Non-Goals

**Goals:**
- Agents can reach the app without a window, and the person always sees and controls that access.
- Layout and viewing changes never block playback or editing.
- Nothing leaves the computer without the person's action.

**Non-Goals:**
- Free-form docking layouts.
- Launch at login and a second-display output.

## Decisions

### D1. Layout presets with splitters
Four presets: Edit (default), Assistant (chat wide), Review (viewer large, timeline small), Vertical (viewer tall for
9:16). Splitters are pointer-drag handles; sizes are fractions, saved per preset in preferences. Backtick maximises the
focused panel and restores it on a second press. During a splitter drag the native viewer rect updates at most once per
animation frame. Alternative: a docking library (large, hard to keep the native viewer aligned).

### D2. Full-screen viewer
A viewer-only mode: the window goes full screen, panels hide, the native viewer rect covers the window. Floating
controls fade after 2 s of no pointer motion; Esc exits. Playback rate is a transport command (`SetRate`) with rates
0.5, 1, 1.25, 1.5 and 2. Audio at rates other than 1× is time-stretched to keep pitch, using a permissively licensed
stretcher chosen during apply; if none passes the licence gate, audio is muted at those rates and the UI says so.
Alternative: OS full screen of the whole editor only (no clean viewing surface).

### D3. Background mode
Setting "Keep running when the window is closed", off by default, offered once when a person connects an agent. When on,
closing the window destroys the web view (frees its memory) and keeps the native process, the control link and the
project engine alive. On macOS the app switches to the accessory activation policy, so it leaves the Dock. The tray
menu lists Open, Recent projects, the project an agent is editing, Pause agent access and Quit. Alternative: hide the
window instead of destroying it (keeps web view memory, misses the idle RAM goal).

### D4. Headless open for agents
The control-link `open` method accepts `background: true`. The app takes the project lock and starts the engine with no
window. `ove app start --background` launches the app in background mode when it is not running. Pause agent access
makes the control link refuse write methods with a clear error until resumed. Alternative: let the CLI edit bundles
offline only (works today, but edits do not appear live and the person cannot see agent activity).

### D5. Notifications
OS notifications for export finished, export failed, generation finished and generation failed, through the Tauri
notification plugin (MIT/Apache). Settings → General has one switch. Alternative: in-app toasts only (not seen in
background mode).

### D6. Aspect-mismatch prompt
When the first visual clip enters a project whose aspect was never chosen, the project adopts the clip's aspect with no
prompt. When the aspect was chosen and the timeline has no visual clip yet, a differing clip shows an inline prompt in
the timeline: "Keep <current>" or "Switch to <clip>", plus "Don't ask again". Switching is one undoable revision. The
"aspect was chosen" flag is per-project UI state, not part of the project document. CLI and MCP imports never prompt;
their result includes an `aspectMismatch` hint. Alternative: a modal sheet that holds the import (blocks agents and
batch imports).

### D7. Onboarding v2
Three screens: (1) what the app does, with a language picker preselected from the OS; (2) assistant choice; (3) "the
editor you know", which sets the keymap preset and layout, plus the privacy points. The answer is stored in local
preferences only. Each checklist step gets "Show me": a spotlight over an element marked with a `data-tour-anchor`
attribute, placed from its bounding box, with a prepare step that opens the needed panel. When the native viewer layer
covers the web layer, the spotlight outlines the viewer instead of dimming it. Alternative: a full guided tour that
moves the person step by step (it blocks people who already know editors).

### D8. Feedback with screenshot
Capture the web view through the platform snapshot call, then paste the engine's current viewer frame into the viewer
rect natively, because the native layer is not in the web view snapshot. Preview before saving: screenshot off by
default, video frame blurred by default, crop handles. "Save report" writes a folder with `report.txt` and
`screenshot.png`; "Copy report" copies the text; an optional prefilled issue link carries text only. The MCP
`report_problem` tool writes a draft that appears in this dialog for review. No OS screen-recording permission is
needed. Alternative: OS screen capture (asks for screen-recording permission and captures other apps).

### D9. What's-new slides
Bundled `whats-new/<version>.json` with translated keys and small images (≤ 1.5 MB per release). Shown once when the
app version is newer than `lastSeenVersion` in preferences; never on a fresh install. Each slide may have "Try it",
which opens the feature. No network fetch. Alternative: load slides from a website (needs network, content changes after
release).

### D10. Activity and usage
A local view grouped by revision source and actor (you, assistant, CLI or MCP agent, template, generation), plus spend
from the existing local cost ledger per project and per chat. It reads revisions and the ledger; it writes nothing.
Alternative: a hosted usage dashboard (needs accounts).

### D11. Light theme
A second token set and "Same as system" in Appearance, on the tokens from add-preferences-i18n-and-keymaps. Clip
colours keep their per-kind values in both themes, with label contrast recomputed. Alternative: invert colours
automatically (poor contrast and wrong accents).

## Risks / Trade-offs

- People forget the app runs in the background. Mitigation: a one-time OS notification when the window first closes in
  background mode, and a tray icon that shows agent activity. Detection: e2e test checks the notification is sent once.
- The native viewer lags behind splitter drags on Windows. Mitigation: freeze the last frame while dragging; the stream
  backend is the fallback. Detection: frame-drop counter in diagnostics during a scripted drag.
- 4K full screen drops frames on the Windows stream backend. Mitigation: drop the preview resolution in full screen
  when drops exceed a threshold. Detection: measured frame drops recorded in Results.
- A spotlight covered by the native layer. Mitigation: outline-only spotlight for the viewer. Detection: screenshot test
  per platform.
- The screenshot leaks client footage. Mitigation: blur on by default and preview before save. Detection: a test that
  the saved image has the viewer region blurred when the default is kept.
- Background mode idle memory too high. Mitigation: destroy the web view on close. Detection: measured RSS with no
  window, target ≤ 120 MB, recorded in Results.

## Migration Plan

New preferences default to current behaviour: background mode off, Edit layout, dark theme, notifications on. No
document change.

## Open Questions

- Waves proceed in master-plan order; this change is wave 4.
- No telemetry and no accounts: feedback and usage stay on this computer.
- Background mode is off by default; whether to offer it automatically after Connect or only in Settings.
- Which time-stretch library passes the licence gate for pitch-kept playback at 1.25× and above.

## Results

(Filled in during apply: measured numbers, screenshots, decisions made.)
