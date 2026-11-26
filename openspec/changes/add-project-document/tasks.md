# Tasks: add-project-document

## 1. Schema and model types

- [x] 1.1 Write `docs/spec/project-schema-v1.md`: every track and item field with type, unit, range, default and
      required flag, checked line by line against docs/spec/document-model.md; owner review; verify every item type
      in the document-model table is covered.
      Status: Written (`docs/spec/project-schema-v1.md`). Owner review: P.1.
- [x] 1.2 Implement model types in `ove-core` (tracks, all item types, media entries, project) with flattened unknown
      fields and microsecond time conversion; verify round-trip tests: hand-written fixtures for every item type, and
      a fixture with unknown fields at every level, serialise back byte-identical in key order.
- [x] 1.3 Generate and commit `docs/spec/project.schema.json`; add CI drift check; verify a deliberately changed field
      fails CI.

## 2. Operations, invariants, inverses

- [x] 2.1 Implement the 10 op kinds with validation and before-value capture; verify unit tests per op covering success
      and each rejection reason.
- [x] 2.2 Implement the invariant checker (unique ids, track/media references, time ranges, finite non-negative values,
      contiguous track order, known enums) with named invariant codes; verify one failing test per invariant code.
- [x] 2.3 Implement batch apply (all-or-nothing on a shared clone) and inverse computation; verify property tests
      (10,000 cases): random valid document + random batch → either rejected with the document unchanged, or applied
      with invariants holding and `apply(inverse(batch))` restoring the original exactly.
- [x] 2.4 Fuzz the JSON reader and batch apply (`cargo fuzz`, 30 min per target in CI nightly); verify no panics,
      only structured errors.
      Status: Targets added (`fuzz/fuzz_targets/read_json.rs`, `apply_batch.rs`), nightly job in `nightly.yml`; 3-minute
      local runs clean. 30-minute runs: P.2.

## 3. Revisions and undo

- [x] 3.1 Implement revision records, monotonic rev (`max(now, prev+1)`), source and summary; verify tests with a
      frozen and a backwards-moving clock.
- [x] 3.2 Implement undo/redo stacks and restore-to-revision (as new revisions); verify scenario tests from the spec,
      including "undo an agent edit" with a 40-op batch and redo clearing after a new batch.
- [x] 3.3 Implement migrations framework with pre-migration copy and read-only open for newer versions; verify with a
      synthetic v0→v1 migration fixture and a v99 document.

## 4. Persistence, recovery and locking (`ove-store`)

- [x] 4.1 Implement bundle create/open/close with folder sanitisation and collision suffixes; verify table tests for
      the sanitisation rules (including Windows reserved names `CON`, `NUL`, `COM1`, which get a suffix) on both OSes.
      Status: Code done (`ove-store` bundle.rs, reserved names get a suffix); table tests pass on Linux. Both OSes: P.3.
- [x] 4.2 Implement the write-ahead revision log, atomic snapshot writer (≤ 1/s, always on close), replay on open,
      retention and periodic snapshots; verify open-after-replay equals in-memory state for 1,000 random batch
      sequences.
- [x] 4.3 Build the crash harness: a child process applies random batches and is killed at random points (including
      inside write/flush/rename via failpoints) 1,000 times per OS; verify every reopen yields a valid document equal
      to the state before or after the last acknowledged batch.
      Status: Done on Linux: 1,000 kills (760 at failpoints, 240 external), every reopen valid. Both OSes: P.3.
- [x] 4.4 Add disk-full, permission-denied and file-held-by-another-process (Windows) tests via an I/O fault layer;
      verify the batch is rejected with the right error code and the previous state is intact.
- [x] 4.5 Implement OS advisory locking with holder info and stale takeover; verify second-writer and stale-lock
      scenarios with two real processes on both OSes.
      Status: Code done (`ove-store` lock.rs); second-writer and stale-lock verified with real processes on Linux. Both
      OSes: P.3.

## 5. Library and media registry

- [x] 5.1 Implement library location (defaults, setting, env var override, network-volume refusal); verify on both OSes
      with a local folder and an SMB share.
      Status: Code done (defaults, env var override, per-OS network-volume detection). Real SMB share: P.4. Settings UI:
      add-settings-and-templates 1.3.
- [x] 5.2 Implement listing via `cache/index.json` with fallback parse and damaged-bundle reporting, rename,
      duplicate and move to Trash/Recycle Bin; verify each library scenario on both OSes.
      Status: Code done (`cache/index.json`, fallback parse, damaged-bundle report, rename, duplicate, Trash). Both
      OSes: P.3.
- [x] 5.3 Implement media registry status on open (online, offline, changed by size/mtime) without modifying the
      document; verify the missing-media scenario.

## 6. Integration checks

- [x] 6.1 Run the performance suite (5,000-item open ≤ 200 ms; 1,000 single-op batches p95 ≤ 10 ms; 10,000-revision
      project open and prune times) on both reference machines; record numbers in design.md Results.
      Status: Done on Linux (open 116 ms, batch p95 1.73 ms; design.md Results). Reference machines: P.5.
- [x] 6.2 Provide `ove-doc` test CLI (create, list, apply-ops-json, history, undo, redo, restore) used by later changes'
      tests; verify an end-to-end script exercising every command passes on both OSes in CI.
      Status: Done (`ove-store/src/bin/ove-doc.rs`, end-to-end test `tests/ove_doc_cli.rs`). Both OSes: P.3.

## Pending verification (macOS, Windows, owner)

Code for the tasks above is done; these checks remain. Each needs a platform, a CI run or the owner.

- [ ] P.1 Owner review of `docs/spec/project-schema-v1.md`.
- [ ] P.2 30-minute nightly fuzz runs of `read_json` and `apply_batch` (needs CI).
- [ ] P.3 Run 4.1, 4.3, 4.5, 5.2 and 6.2 on macOS and Windows (CI jobs exist; blocked like harden-for-alpha 6.5).
      Windows: done 2026-11-26 (workspace tests, crash harness 1,000 kills). Left: macOS.
- [ ] P.4 Network-volume refusal with a real SMB share on macOS and Windows.
- [ ] P.5 Performance suite on the macOS and Windows reference machines; record in Results.
      Windows: done 2026-11-26 (design.md Results). Left: macOS.
