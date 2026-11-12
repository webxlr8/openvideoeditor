# Tasks: harden-for-alpha

## 1. Contained failures

- [x] 1.1 Every Tauri command runs inside panic containment; async commands go through `worker::blocking`, `guarded`
      or `projects::serial`. Verified: the mock-runtime test drives the real invoke handler; panicking sync and async
      commands return `worker_panic` and the next command answers. A source test fails if an async command skips the
      helpers.
- [x] 1.2 Poisoned locks recover; the open project is re-validated and reloaded from the revision log if needed.
      Verified: `edit_after_a_panicking_lock_holder_succeeds`.
- [x] 1.3 A dead playback engine restarts (at most 3 restarts in 60 s, then a clear error). Verified manually; no
      unit test (needs a real app handle).
- [x] 1.4 Project, file, keychain and network work moved off the main thread; media status and thumbnails come from
      a per-project cache refreshed off-thread. Verified: media-cache test; code review.
- [x] 1.5 Control socket and MCP server contain handler panics; control connections capped at 32. Verified: tests in
      `ove-control` and `ove-cli`.
- [x] 1.6 Generation runs contained; stale jobs are reaped; failures clean up placeholders with the project closed.
      Verified: `ove-generate` tests.
- [x] 1.7 React error boundaries per region plus `onCaughtError`/`onUncaughtError`; type-aware ESLint with
      `no-floating-promises`. Verified: `ErrorBoundary.test.tsx`; lint 0 errors, 0 warnings.

## 2. No silent loss

- [x] 2.1 Export writes `<name>.partial.<ext>` and renames on success; failures and cancels leave the earlier file.
      Verified: `ove-render/tests/export_safety.rs` (6 tests).
- [x] 2.2 Offline media refuses export by name. Verified: `offline_media_refuses_export_by_name`.
- [x] 2.3 Export reports a fallback encoder (anything not hardware or H.264). Verified: shows on the Linux build
      (`mpeg4`).
- [x] 2.4 Interchange export lists what will not carry over, before writing (dialog, CLI, JSON). Verified:
      `ove-interchange` report tests, Export dialog tests.
- [x] 2.5 `captionComposition.fallback: hide` honoured. Verified: render test.

## 3. Secrets

- [x] 3.1 Keys redacted in every log layer; HTTP internals capped at `warn`. Verified: a real ureq request with a
      key header at debug level leaves no key in the output.
- [x] 3.2 fal key only to fal hosts over https; keyed agents do not follow redirects. Verified: `tests/fal_hosts.rs`.
- [x] 3.3 Endpoint overrides only in debug builds or with `OVE_DEV_ENDPOINTS=1`, https or localhost only.
- [x] 3.4 Ids validated before becoming paths; MCP scratch files in a private temp directory.
- [x] 3.5 Linux keyring persistent (Secret Service behind the kernel keyring).
- [x] 3.6 Control-link revision source limited to `cli`/`mcp`.

## 4. Media correctness

- [x] 4.1 PQ/HLG tone-mapped to SDR (BT.2390, 203-nit reference white) in the compositor (CPU, table-driven) and in
      the wgpu viewer shader. Verified: GPU, CPU and compositor agree within 1 code value on PQ and HLG fixtures.
- [ ] 4.2 Desktop viewer uses the GPU HDR path (`OutputFormat::DisplayHdr` + `upload_with_transfer`). HDR already
      displays correctly through the CPU path; needs verification on macOS/Windows hardware (phase 1).
- [x] 4.3 Mixdown: −1 dBFS soft-knee limiter; band-limited speed changes (rubato). Verified: limiter and aliasing
      tests.
- [x] 4.4 Transcription in 30 s windows with 2 s overlap; desktop captions use `transcribe_file`; stable SHA-256
      cache keys. Verified: streamed transcript equals whole-file transcript; 75 s clip across a seam.

## 5. Honest UI

- [x] 5.1 Templates only ask for what the app does. Verified: `templates.test.ts`.
- [x] 5.2 Audio buttons say what they do; region and behind-subject fallbacks are labelled.
- [x] 5.3 The assistant's `seek_viewer` moves the viewer. Verified: chat test lands on 00:00:01:15.
- [x] 5.4 Chat panel opens by default only when a key is stored.

## 6. Verification

- [x] 6.1 Test media generated under an exclusive lock with atomic writes; one generator run per process. Verified:
      9 media test binaries started at once from an empty folder, twice, all green.
- [x] 6.2 Tests that need media tools fail when the tool is missing (`OVE_ALLOW_MISSING_MEDIA_TOOLS=1` to skip).
- [x] 6.3 CI lints and tests the whole workspace on macOS, Windows and Ubuntu.
- [x] 6.4 `cargo deny check` passes: targets scoped to macOS and Windows; two reviewed unmaintained notices recorded.
- [ ] 6.5 One green CI run on all three OSes (blocked: GitHub Actions jobs do not start on this account).
- [x] 6.6 TLS through the operating system's verifier (`rustls-platform-verifier`, `ove_guard::http`) for every
      outgoing request, so downloads work behind corporate TLS proxies. Verified: the network test passes behind a
      TLS-inspecting proxy that the bundled root list rejects.
