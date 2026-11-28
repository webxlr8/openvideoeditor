# Spec Delta

## Purpose

Interface language and preferences: one durable preferences store, translated interface and native strings, live
language change, right-to-left layout, pseudo-locales, and a boundary that keeps agent-facing contracts in English.

## ADDED Requirements

### Requirement: Durable preferences store
The app SHALL keep user preferences (language, shortcuts, theme tokens, clip colours, notification and privacy
choices) in one native preferences file that both the interface and the native shell read. Preferences SHALL survive
clearing the web view data. On the first start after this change, existing shortcut and crash-report choices SHALL be
moved into the file once.

#### Scenario: Survives web data reset
- **WHEN** a person rebinds a shortcut, quits, clears the web view data and starts the app
- **THEN** the rebound shortcut still works

#### Scenario: One-time migration
- **WHEN** the app starts for the first time after the update and old shortcut settings exist
- **THEN** the settings appear in the preferences file and the old copies are removed

#### Scenario: Corrupt preferences file
- **WHEN** the preferences file contains invalid content
- **THEN** the app starts with default preferences, keeps a copy of the invalid file, and shows a notice once

### Requirement: Interface language
The person SHALL choose the interface language in Settings → General, with "Same as system" as the default. A change
SHALL apply at once to the interface and to native menus, without a restart. When a string has no translation, the
English text SHALL appear.

#### Scenario: Live change
- **WHEN** a person switches the language from English to German with a project open
- **THEN** panels, dialogs and the native menu bar show German text without a restart, and the project stays open

#### Scenario: Missing translation
- **WHEN** a string has no German translation
- **THEN** the English string appears in its place and no key name is shown

#### Scenario: System default
- **WHEN** the language is "Same as system" and the OS language is Japanese
- **THEN** the interface is Japanese at first start

### Requirement: Plural and placeholder forms
Translated strings SHALL support plural and select forms per language and named placeholders. Error messages SHALL be
shown in the interface language by their error code, with the English message as fallback.

#### Scenario: Plural form
- **WHEN** a status line reports 1 item and then 5 items in Polish or Arabic
- **THEN** each count uses the plural form that language requires

#### Scenario: Untranslated error
- **WHEN** an error occurs whose code has no translation
- **THEN** the English error message appears with the code

### Requirement: Agent contracts stay in English
Tool names, tool descriptions, CLI output, MCP messages, the projection file and the agent system prompt SHALL be the
same bytes in every interface language. The assistant SHALL receive a hint to reply in the interface language.

#### Scenario: Projection unchanged by language
- **WHEN** the same project is printed with the interface in English and in Japanese
- **THEN** both `timeline.tsx` files are byte-identical

#### Scenario: Assistant reply language
- **WHEN** the interface is French and a person writes to the assistant in English
- **THEN** the request to the model includes a hint to reply in French unless the person asks otherwise

### Requirement: Right-to-left layout
For a right-to-left language the window layout SHALL mirror. The timeline, ruler, waveforms, timecode fields and
transport controls SHALL keep time running left to right.

#### Scenario: Arabic layout
- **WHEN** the interface language is Arabic
- **THEN** side panels and text align right to left, and dragging a clip to the right still moves it later in time

### Requirement: Pseudo-locales in development builds
Development builds SHALL offer an expanded accented pseudo-locale and a mirrored pseudo-locale. Release builds SHALL NOT
offer them.

#### Scenario: Untranslated text visible
- **WHEN** a development build runs in the expanded pseudo-locale
- **THEN** every translated string appears bracketed and accented, and any plain string is an extraction miss

#### Scenario: Hidden in release
- **WHEN** a person opens the language list in a release build
- **THEN** no pseudo-locale is listed

### Requirement: Wave-1 locales
The release SHALL include Spanish, Brazilian Portuguese, German, French, Japanese, Simplified Chinese and Arabic, each
complete for every interface string and reviewed by a fluent reviewer.

#### Scenario: Locale complete
- **WHEN** the string check runs on a release branch
- **THEN** it reports zero missing keys and zero placeholder mismatches for each wave-1 locale, or the build fails
