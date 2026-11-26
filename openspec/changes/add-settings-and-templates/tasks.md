# Tasks: add-settings-and-templates

## 1. Implementation

- [x] 1.1 Settings page with tests.
- [x] 1.2 Templates page and dialog with tests.
- [x] 1.3 Choosing another library folder from Settings (environment variable today).
      Done: Settings → Library (`set_library`, kept in `library.json` for the app and the command line; OVE_LIBRARY
      still wins). Verified: `ove-store/tests/library_setting.rs`, `Settings.test.tsx`.
- [x] 1.4 Installing the command line into PATH from Settings.
      Done on Windows (user Path in HKCU\Environment, no administrator); macOS shows the command to run. Verified:
      `cli_install.rs` tests (path lists; the registry round trip under a test key), `Settings.test.tsx`.
