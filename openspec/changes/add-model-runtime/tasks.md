# Tasks: add-model-runtime

## 1. Runtime spike

- [x] 1.1 Export Silero VAD and Beat This (small) to ONNX with our own scripts and record their weight licences; verify
      each export matches the reference implementation (max absolute logit difference < 1e-3 on 3 fixtures).
      Done: `tools/model-spike/export_models.py`; worst difference 1.64e-4; licences in design.md R1, parity in R2.
- [ ] 1.2 Run both models under the dynamically loaded ONNX runtime and the pure-Rust runtime on macOS and Windows;
      verify and record in design.md Results: loads yes/no, parity, real-time factor, peak RSS, added binary size.
      Windows measured (design.md R3, `tools/model-spike/bench`); the macOS row is still open.
- [ ] 1.3 Record the decision (D4) with the numbers; verify the owner signs off in the change notes.
      Recommendation recorded in design.md R4 (dynamically loaded runtime); waiting for the owner's sign-off.

## 2. Manifests, downloader and store

- [x] 2.1 Create crate `ove-models` with the manifest type and compiled-in manifests; verify unit tests reject a
      manifest with a moving-branch URL or a missing hash.
      Done: `crates/ove-models/src/manifest.rs` tests; the whisper URLs now name commit 5359861c.
- [x] 2.2 Implement the downloader (Range resume, streaming hash, partial file, rename, space check, cancel); verify
      tests against a local HTTP server: good, corrupt, interrupted-then-resumed, no-space and cancel cases.
      Done: `crates/ove-models/tests/store.rs` (12 tests, cancel measured under 1 s).
- [x] 2.3 Implement the store layout and `installed.json`, and adopt existing whisper files in place; verify an upgrade
      test with whisper files present does not download and captions still run.
      Done: `crates/ove-transcribe/tests/upgrade.rs` (run with `--ignored` and the tiny model; passed on Windows).
- [x] 2.4 Move `ove-transcribe` model handling onto `ove-models`; verify the existing transcribe tests pass unchanged.
      Done: unit tests pass; `tests/speech.rs` is unchanged (it is ignored without espeak-ng); `tests/windows.rs`
      needs the Unix-only media fixture script, so it was not run on this Windows machine.
- [x] 2.5 Document the store and manifests in `docs/cli/models.md`; verify the docs' examples run in the CLI tests.
      Done: `crates/ove-cli/tests/models.rs` runs the page's 5 examples with their exit codes; `ove docs models`.

## 3. Inference adapter and worker

- [x] 3.1 Implement the inference trait for the chosen runtime (and the second runtime if D4 keeps both); verify the
      two spike models run through it with recorded parity.
      Done with the recommended runtime (design.md R4; the owner's sign-off is still open): `ove_models::infer` and
      `ove_models::onnx`; `spike_models_run_through_the_adapter_with_parity` in `crates/ove-cli/tests/model_worker.rs`
      (in process and in the worker, parity in design.md R5).
- [x] 3.2 Implement `ove model-worker` with the stdio protocol, lazy load, 120 s idle exit and restart on crash; verify
      tests: idle exit, killed worker fails the job and the next job succeeds, 256×256×3 tensor round trip ≤ 2 ms.
      Done: `crates/ove-cli/tests/model_worker.rs`; round trip median 0.56 ms (design.md R5).
- [ ] 3.3 Implement on-demand download of the native runtime library (if chosen) through the consent gate; verify a
      clean-profile test reports "runtime not installed" and installs after consent.
      Open: needs the D4 sign-off and a pinned, immutable location for the library on both platforms. The upstream
      release page could not be reached at its old address during apply; the plan is to mirror the exact library
      files as versioned release assets of this project (`add-distribution`) and pin those. The worker already
      takes the library path (`OVE_ONNXRUNTIME_LIB`) and reports a missing runtime per job.

## 4. Consent and surfaces

- [ ] 4.1 Implement the consent gate (`NeedsConsent` over 50 MB, origin-aware); verify an MCP call for a large model
      returns the result and downloads nothing, and a small model downloads with progress.
- [x] 4.2 Add `ove models list|install|remove` with `--json` and `--yes`; verify CLI tests for success and refusal paths
      and exit codes.
      Done (ahead of group 3, because 2.5 needs the command): `crates/ove-cli/tests/models.rs`; the success path
      uses the real tiny model from a local mirror (`--ignored` with `OVE_WHISPER_MODEL_DIR`; passed on Windows).
- [ ] 4.3 Add MCP `list_models_local`; verify an MCP contract test.
- [ ] 4.4 Add Settings → Storage "Models" (list, sizes, Remove, wait-while-in-use); verify a UI test removes a model
      and the feature then reports it missing.

## 5. Integration checks

- [ ] 5.1 Clean-profile run in the built app on macOS and Windows: install a small and a large model with consent, run
      inference, remove both; verify the flows work and record download and load times in Results.
- [ ] 5.2 Memory: with a large model loaded and then idle for 2 minutes, measure editor and worker RSS on both
      machines; verify the editor grows ≤ 50 MB during use and the worker has exited after idle.
- [ ] 5.3 Installer size: verify the installer is unchanged within 1 MB of the previous release on both platforms.
