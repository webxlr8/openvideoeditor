# Design: add-preferences-i18n-and-keymaps

## Context

See proposal.md (Why). Today the UI depends only on React and the Tauri API. Strings are inline English. `ui/index.html`
sets `lang="en"`. The CSS has about 79 left/right properties and about 160 hex colour literals. `ui/src/keys.ts` has 9
single-key actions stored in web storage under `ove.shortcuts`; modifier combinations are hard-coded. Onboarding state
is a native JSON file. Requirements: specs/localization, specs/keymaps.

## Goals / Non-Goals

**Goals:**
- One source of truth for every string, every shortcut and every colour token.
- Language change without restart, in the UI and in native menus.
- Agent-facing contracts stay byte-identical in every locale.

**Non-Goals:**
- Translating agent prompts, tool descriptions, CLI output or `timeline.tsx`.
- A light theme (add-workspace-modes-and-background-access).

## Decisions

### D1. Preferences store: one native JSON file
A `preferences.json` file in the app config folder, beside `onboarding.json`. The native side owns reads and writes
(atomic write: temp file, then rename). The UI reads it at start and subscribes to change events. First start after
this change copies the web-storage keys `ove.shortcuts` and the crash opt-in into the file, then deletes them.
Alternatives: keep web storage (lost when web data is cleared, and native menus cannot read it); a SQLite file
(too heavy for a few dozen keys).

### D2. Message format: ICU MessageFormat behind a typed `t()`
Use `intl-messageformat` (BSD-3, about 10 KB gzip) behind our own `t(key, args)` hook. A build step generates a
TypeScript key union from `locales/en.json`, so an unknown key fails the type check. Alternatives: i18next with
react-i18next (larger, non-ICU plural syntax); Lingui (needs a compiler macro and build plugin); our own ICU parser
(plural rules for many languages are easy to get wrong).

### D3. Native strings: same JSON, simple substitution only
The native shell loads the same locale JSON. It supports only `{name}` substitution. A lint refuses plural or select
syntax in keys used by native code. Alternative: Fluent in Rust (a second message format to maintain).

### D4. Errors translated by code
Native errors already carry an `ErrorCode`. The UI maps the code to a key `error.<code>` and falls back to the English
message from the native side when the key is missing. Alternative: translate in Rust (the native side would need the
UI's plural support).

### D5. Agent-contract boundary
`scripts/i18n-check.mjs` fails when `t(` or a locale key appears in `crates/ove-agent`, `crates/ove-cli`,
`crates/ove-projection` or in MCP tool descriptions. The same script checks placeholder parity, missing keys and unused
keys. Alternative: review only (drifts over time).

### D6. Pseudo-locales
`en-XA`: every string accented and padded by about 35%, wrapped in brackets, so untranslated strings and clipping are
visible. `ar-XB`: mirrored right to left. Both exist only in development builds. Alternative: wait for real
translations (layout bugs appear late and per language).

### D7. Right-to-left
Set `dir` on `<html>` from the locale. Move CSS to logical properties (`margin-inline-start` and similar). The timeline,
ruler, waveform, timecode fields and the viewer transport stay `dir="ltr"`, because time runs left to right in every
editor. Alternative: mirror everything (time reversed, confusing for editors trained on any NLE).

### D8. Keymap registry
`ui/src/keymap/registry.ts` lists each action with id, category, label key and default chord. A chord is
`{key, code, mod, shift, alt}`; `mod` means Command on macOS and Ctrl on Windows. Presets are override maps on top of
Default. Matching uses `KeyboardEvent.key` for Latin layouts and `KeyboardEvent.code` when `key` is not a Latin letter,
so Cyrillic and Greek layouts still trigger letter shortcuts. The shortcut overlay and native menu accelerators are
both generated from the registry. Alternative: keep menu accelerators hand-written (they drift from the overlay).

### D9. Presets
Default, Premiere-style, Resolve-style, Final Cut-style, CapCut-style, with the overrides listed in
docs/spec/presets.md (Keyboard shortcuts). All other entries use Default. Preset files are written from our own action
list; we copy no vendor documentation. A reserved-chord table per OS refuses chords that the OS owns (for example
Command+H and Command+M on macOS, Ctrl+Alt+Delete on Windows). Alternative: full vendor-like maps (copy risk, and
actions we do not have yet).

### D10. Theme tokens and clip colours
Replace hex literals with CSS custom properties grouped as surface, text, accent, state and clip kind. Clip colours are
seven tokens, one per kind, editable in Appearance and stored in preferences. A preset palette based on the public
Okabe–Ito set gives a colour-blind-safe option. Label text colour is chosen black or white from WCAG contrast.
Alternative: per-clip custom colours stored in the document (changes the document contract; later if needed).

### D11. Locale choice
Wave 1: es, pt-BR, de, fr, ja, zh-Hans and ar (ar proves right-to-left). The first translation is machine-assisted and
reviewed by a fluent reviewer per locale before release. Alternative: more locales at once (review capacity is the
limit).

## Risks / Trade-offs

- German and CJK text overflows controls. Mitigation: `en-XA` screenshot runs of the app e2e suite. Detection: the
  screenshot diff and an overflow check on fixed-width controls.
- A missed string stays English. Mitigation: the string check plus the bracketed pseudo-locale. Detection: any
  unbracketed text in an `en-XA` screenshot fails the run.
- Preset chords clash with OS shortcuts. Mitigation: the reserved-chord table. Detection: a test per OS that presses
  each preset chord in the built app.
- Migration of web-storage keys fails on a corrupt value. Mitigation: invalid values fall back to defaults and are
  logged. Detection: a migration test with corrupt input.
- Extracting about 2,000 strings causes regressions. Mitigation: extract one area per task with screenshots before and
  after. Detection: screenshot diff in `en`.

## Migration Plan

First start after the change: read web-storage keys, write them into `preferences.json`, delete the web keys. Running
an older build after that uses default shortcuts; nothing else is lost.

## Open Questions

- Waves proceed in master-plan order; this change is wave 3.
- No telemetry and no accounts: translation quality is checked by reviewers, not by usage data.
- Who reviews each wave-1 locale before release.
- Whether the keymap file format is published as a public contract for agents.

## Results

(Filled in during apply: measured numbers, screenshots, decisions made.)
