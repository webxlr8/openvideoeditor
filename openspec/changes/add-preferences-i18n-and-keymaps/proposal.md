# Proposal: add-preferences-i18n-and-keymaps

## Why

The interface is English only, and every string is hard-coded in the UI and in native menus. People who edit in
another language cannot use the app comfortably. Keyboard shortcuts cover only 9 single-key actions. The 27 actions
and 5 presets in the reference spec are not built, so people who come from other editors must relearn every key.
Settings are split between web storage and native files, so clearing the web view data loses shortcuts and opt-ins.
The theme uses about 160 hard-coded colours, so clip colours and a later light theme cannot change.

## What Changes

- One native preferences store, read by the UI and by the native shell. Existing web-storage keys move into it once.
- A localization runtime for the UI and for native strings (menus, tray, dialogs, notifications), with plural and
  select forms, live language change with no restart, and error messages translated by error code.
- A string check: missing keys, unused keys, placeholder mismatches between locales, and a ban on translated strings in
  agent-facing contracts (tool names, tool descriptions, CLI output, `timeline.tsx`).
- Two pseudo-locales in development builds: one longer and accented, one mirrored right to left.
- Right-to-left layout: the window mirrors, but the timeline, ruler, waveforms and timecodes stay left to right.
- Wave-1 locales: Spanish, Brazilian Portuguese, German, French, Japanese, Simplified Chinese and Arabic.
- The assistant gets a hint to reply in the interface language.
- A keymap registry with the 27 actions of the reference spec and 5 presets (Default, Premiere-style, Resolve-style,
  Final Cut-style, CapCut-style). Each preset map is our own design. It keeps only the overrides the reference spec
  lists.
- Shortcuts settings: search, conflict warnings, reset per action, and import and export of a keymap file. The shortcut
  overlay and the native menu accelerators come from the same keymap.
- Letter shortcuts work on non-Latin keyboard layouts.
- Theme colours become tokens. Clips get one colour per kind (video, audio, image, text, caption, motion, generated),
  with a colour-blind-safe preset and readable label contrast.
- New Settings panes: General (language, notifications) and Appearance (theme tokens, clip colours).

Out of scope: wave-2 locales, the light theme and layout presets (add-workspace-modes-and-background-access), the tray
and background mode, onboarding changes, hosted community translation, translation of agent prompts or tool contracts.

References: docs/spec/presets.md (Keyboard shortcuts), docs/spec/desktop-shell.md (menus and settings).

## Capabilities

### New Capabilities
- `localization`: preferences store, interface language, translated UI and native strings, pseudo-locales,
  right-to-left layout, the string check, and the agent-contract boundary.
- `keymaps`: the action registry, presets, rebinding, conflicts, keymap files, menu accelerators, layout-independent
  letter keys, and per-kind clip colours with theme tokens.

### Modified Capabilities
- None.

## Impact

- UI: new `ui/src/i18n/`, `ui/src/keymap/` and `locales/` folders; `keys.ts`, `Shortcuts.tsx`, `Settings.tsx` and the
  styles change. Most UI files change once, to move strings into keys.
- Native shell: menu builder, tray strings (used later by add-workspace-modes-and-background-access) and dialogs read
  translated strings; a new preferences file next to the onboarding file.
- New UI dependency for message formatting; licence checked by the JS licence gate.
- Later changes that add UI strings must add keys. add-workspace-modes-and-background-access depends on the
  preferences store and the native strings from this change.
