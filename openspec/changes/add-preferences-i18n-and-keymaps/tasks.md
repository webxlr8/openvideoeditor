# Tasks: add-preferences-i18n-and-keymaps

## 1. Preferences store

- [ ] 1.1 Add the native preferences file (atomic write, change events, invalid-file fallback with a kept copy); verify
      unit tests for write, reload, corrupt input and concurrent writes.
- [ ] 1.2 Move web-storage shortcut and crash opt-in keys into the file on first start and delete the old keys; verify a
      migration test with valid, missing and corrupt old values.
- [ ] 1.3 Expose preferences to the UI through typed commands and a subscription; verify a UI test that a change made
      in one window appears in another.

## 2. Localization runtime

- [ ] 2.1 Add the typed `t()` hook with ICU plural and select support and the generated key union; verify `tsc` fails on
      an unknown key and unit tests cover plural rules for en, pl and ar.
- [ ] 2.2 Load the same locale files in the native shell with `{name}` substitution, and rebuild native menus on
      language change; verify a native unit test and a check that the menu labels change without restart.
- [ ] 2.3 Translate errors by code with English fallback; verify a test for a translated code and an untranslated code.
- [ ] 2.4 Add the "reply in the interface language" hint to assistant requests; verify an agent test with a mock model
      inspects the request.
- [ ] 2.5 Document how to add strings and locales in `docs/`; verify the documented commands run in CI.

## 3. String check and pseudo-locales

- [ ] 3.1 Write `scripts/i18n-check.mjs` (missing, unused, placeholder parity, plural ban in native keys, ban on
      translation calls in agent crates and MCP descriptions); verify fixture tests for each failure and add it to CI.
- [ ] 3.2 Generate the expanded and mirrored pseudo-locales in development builds only; verify the release build's
      language list has no pseudo-locale.
- [ ] 3.3 Add a projection byte-identity test across two interface languages; verify it passes.

## 4. String extraction and right-to-left

- [ ] 4.1 Extract strings area by area (Home, Editor panels, Inspector, Settings, dialogs, onboarding, menus); verify
      per area an `en` screenshot diff with no change and an `en-XA` screenshot with no plain text.
- [ ] 4.2 Move CSS to logical properties and set `dir` from the locale, with left-to-right islands for timeline, ruler,
      waveforms, timecodes and transport; verify `ar-XB` screenshots and a UI test that dragging right moves a clip later.

## 5. Wave-1 locales

- [ ] 5.1 Add es, pt-BR, de, fr, ja, zh-Hans and ar translations and record the reviewer per locale; verify the string
      check reports zero missing keys and zero placeholder mismatches.
- [ ] 5.2 Fix overflow found in de and ja screenshots; verify the overflow check passes on fixed-width controls.

## 6. Keymaps

- [ ] 6.1 Build the action registry with the 27 reference actions and platform `mod` mapping; verify unit tests per
      action default and a test that each default chord triggers its action.
- [ ] 6.2 Add the 5 presets, the reserved-chord table per OS and "Reset to preset"; verify unit tests for each preset's
      listed overrides and for refusal of reserved chords.
- [ ] 6.3 Build Settings → Shortcuts (search, rebind, conflict swap or cancel, reset per action, import and export);
      verify UI tests for conflict, reserved chord, invalid file and a macOS-to-Windows round trip.
- [ ] 6.4 Generate native menu accelerators and the overlay from the keymap; verify a test that a rebind updates both.
- [ ] 6.5 Match letter keys by physical position on non-Latin layouts; verify a unit test with Cyrillic key events.
- [ ] 6.6 Document actions and presets in `docs/`; verify the doc list matches the registry in a CI check.

## 7. Theme tokens and clip colours

- [ ] 7.1 Replace hex colour literals with tokens; verify a lint that finds no hex literal outside the token file and
      an unchanged dark-theme screenshot.
- [ ] 7.2 Add per-kind clip colour tokens, the colour-blind-safe preset and contrast-based label colour; verify an axe
      contrast check passes for every preset and a test that colours persist after restart.
- [ ] 7.3 Add Settings → General (language, notifications) and Settings → Appearance (clip colours); verify UI tests.

## 8. Integration checks

- [ ] 8.1 In the built app on macOS and Windows: switch between en, de, ja and ar with a project open; verify panels,
      dialogs, menus and the tray strings change without restart and the timeline stays left to right.
- [ ] 8.2 In the built app on macOS and Windows: apply each preset and press split, export and fullscreen; verify each
      triggers and no OS shortcut is captured.
- [ ] 8.3 Measure cold start to Home with the localization runtime loaded on both reference machines; verify it stays
      ≤ 1.5 s and record the number in design.md Results.
