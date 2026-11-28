# Tasks: add-workspace-modes-and-background-access

## 1. Layouts and splitters

- [ ] 1.1 Add the four layout presets, splitters with per-preset sizes in preferences, and maximise/restore; verify UI
      tests for size memory per preset and maximise twice.
- [ ] 1.2 Throttle native viewer rect updates during splitter drags to one per animation frame, and freeze the frame on
      the stream backend while dragging; verify frame drops during a scripted drag and record them in design.md Results.
- [ ] 1.3 Suggest the Vertical preset once for 9:16 projects; verify a UI test for accept and decline.

## 2. Full-screen viewer and rates

- [ ] 2.1 Add the full-screen viewer mode with floating controls, auto-hide and Esc; verify a UI test that the playhead
      is the same on exit.
- [ ] 2.2 Add the transport rate command with 0.5, 1, 1.25, 1.5 and 2, its control-link method, and the rate indicator;
      verify unit tests for timing at each rate and a control-link test.
- [ ] 2.3 Choose and add a licence-approved pitch-kept time stretcher, or mute audio at non-1× rates with a notice;
      verify the licence gate and an audio test at 1.5×.

## 3. Background access

- [ ] 3.1 Add the background-mode setting, tray icon and menu, web view teardown on close and the macOS accessory
      policy; verify on both OSes that `ove app context` works with the window closed and quit works from the tray.
- [ ] 3.2 Add `background: true` to the control-link `open` method and `ove app start --background`; verify CLI tests
      for open, edit, reopen with window, and lock-held refusal.
- [ ] 3.3 Add Pause agent access with refusal of write methods; verify a control-link test for paused and resumed.
- [ ] 3.4 Add finish notifications with the General switch; verify notification calls in tests for export and
      generation success and failure, and none when switched off.
- [ ] 3.5 Document background mode and the new CLI flag in `docs/cli/`; verify the examples run in the CLI tests.

## 4. Aspect-mismatch prompt

- [ ] 4.1 Adopt the aspect silently when never chosen; show the inline keep/switch prompt otherwise, with Don't ask
      again; verify UI tests for both paths and one-step undo.
- [ ] 4.2 Return an `aspectMismatch` hint from CLI and MCP imports; verify a CLI test and an MCP contract test.

## 5. Onboarding v2

- [ ] 5.1 Rebuild the welcome as language, assistant and "the editor you know" screens that set preset and layout
      locally; verify an e2e first-run test on a clean profile and that no request carries the answer.
- [ ] 5.2 Add "Show me" spotlights with anchors and prepare steps, with outline-only mode over the native viewer; verify
      screenshot tests per platform.

## 6. Feedback, what's new, activity

- [ ] 6.1 Capture the web view snapshot and composite the viewer frame; add preview with blur on by default, crop, save
      folder and copy; verify the saved image has the viewer region blurred by default and no OS permission prompt.
- [ ] 6.2 Add the MCP `report_problem` draft that opens in the dialog; verify an MCP test that nothing is saved before
      the person acts.
- [ ] 6.3 Add bundled what's-new slides with `lastSeenVersion` and "Try it" links; verify upgrade shows them once and a
      fresh install shows none.
- [ ] 6.4 Add the local Activity and Usage view from revisions and the cost ledger; verify totals match per-turn costs
      in a test project.
- [ ] 6.5 Add the Settings → Storage pane frame that lists entries registered by other changes; verify a UI test with a
      stub entry.

## 7. Light theme

- [ ] 7.1 Add the light token set and Same as system, with clip label contrast recomputed; verify the axe contrast check
      passes in both themes and a test that an OS theme change applies without restart.

## 8. Integration checks

- [ ] 8.1 In the built app on macOS and Windows: close the window in background mode, have an MCP agent open a project
      and edit it, see the tray entry, pause access, then reopen the window; verify each step behaves as specified.
- [ ] 8.2 In the built app on macOS and Windows: play 4K media in the full-screen viewer at 1× and 1.5×; verify frame
      drops and record them in design.md Results.
- [ ] 8.3 Measure idle RAM with no window in background mode on both reference machines; verify ≤ 120 MB and record it.
- [ ] 8.4 First run on clean profiles on both OSes, timed from start to first export; record the time in Results.
