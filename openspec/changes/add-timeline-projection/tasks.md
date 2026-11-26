# Tasks: add-timeline-projection

## 1. Projection format

- [x] 1.1 Write `docs/spec/projection-format.md`: header layout, element and attribute order per item type, number and
      string printing rules, default omission, literal grammar, diagnostic codes; verify every rule in
      docs/spec/document-model.md (Projection rules) maps to a section, reviewed by the owner.
      Status: Written (`docs/spec/projection-format.md`). Owner review: P.1.
- [x] 1.2 Build the golden corpus: hand-written documents for every track and item type with their expected
      projections, plus invalid projections with expected diagnostics; verify the corpus covers each diagnostic code.

## 2. Printer and typings

- [x] 2.1 Implement the printer (header, tracks, items, attribute order, number/string rules, default omission); verify
      golden tests pass byte-for-byte on macOS and Windows.
      Status: Done; golden corpus byte-for-byte on Linux. Both OSes: P.2.
- [x] 2.2 Generate `timeline.d.ts` from the document type definitions with a CI drift check; verify the corpus
      projections type-check against it with the TypeScript compiler.
- [x] 2.3 Rewrite the projection after every revision in `ove-store`; verify a test applying batches from different
      sources sees the header `rev` follow each new revision.

## 3. Parser and checker

- [x] 3.1 Implement the oxc-based parser into the projection model with literal-only enforcement and line/column
      diagnostics; verify all invalid-corpus cases report the expected code at the expected position.
- [x] 3.2 Implement unknown-attribute round trip and unknown-key-in-object warnings; verify with corpus cases.
- [x] 3.3 Property test round trip (10,000 generated documents): `parse(print(D)) == normalise(D)` and print idempotence;
      verify passes on both OSes.
      Status: Done on Linux (10,000 documents). Both OSes: P.2.
- [x] 3.4 Fuzz the parser (nightly, 30 min); verify no panics and every input yields a result or diagnostics.
      Status: Target added (`fuzz/fuzz_targets/parse_projection.rs`), nightly job; 3-minute local run clean. 30-minute
      run: P.3.

## 4. Merge and apply

- [x] 4.1 Implement base reconstruction, normalised diff (field level, objects one key deeper, arrays whole) and the
      change set; verify unit tests per change kind.
- [x] 4.2 Implement the rebase rule, conflict policies and reporting, removal and re-creation cases; verify every
      scenario in specs/timeline-projection (merge, granularity, conflict policy) as a named test.
- [x] 4.3 Implement merge properties 2–7 from design.md D5 as property tests (10,000 cases each); verify all pass.
- [x] 4.4 Apply the merged result as one batch; empty change set creates no revision; base revision availability
      checks; verify pruned/missing/newer rev refusals and the no-op scenarios.
- [x] 4.5 Implement text-edit apply and whole-source apply; verify ambiguous, overlapping, deleting and inserting edits.

## 5. Media references, placeholders, verify

- [x] 5.1 Implement `src` resolution and new-path registration (`awaiting-probe`, de-duplicated names); verify the media
      reference scenarios with real files on both OSes, including `~/` and `..` paths and Windows drive paths.
      Status: Code done (`ove-projection` read.rs `resolve_src`, `~/` and `~\`); scenario tests pass on Linux. Real
      files and drive paths: P.4.
- [x] 5.2 Implement pending placeholders (create, count, fill keeping id); verify the fill scenario.
- [x] 5.3 Implement the lint registry and the five structural lints; verify one positive and one negative test per lint
      and the clean/pending verify scenarios.

## 6. CLI

- [x] 6.1 Implement `ove source print|check|apply`, `verify`, `history`, `undo [rev]` with project resolution, `--json`,
      exit codes and lock-held refusal; verify a CLI integration test suite runs each command's success and refusal
      paths on both OSes.
      Status: Done; CLI suite and documented examples pass on Linux. Both OSes: P.2.
- [x] 6.2 Document the commands in `docs/cli/` (usage, JSON shapes, exit codes); verify the docs' examples are executed
      by the integration tests.

## 7. Integration checks

- [x] 7.1 Concurrency soak: one process applies random GUI-style batches while another prints, edits and applies
      projections for 30 minutes; verify zero lost edits (checked against an oracle model) and zero invariant failures.
      Status: Soak test in `ove-store/tests/projection.rs`; 60 s run clean. 30-minute run: P.3.
- [x] 7.2 Performance: 5,000-item print ≤ 100 ms and one-item check + apply ≤ 150 ms on both reference machines;
      record numbers in design.md Results.
      Status: Done on Linux (print 28 ms, check + apply 61 ms). Reference machines: P.5.

## Pending verification (macOS, Windows, owner)

Code for the tasks above is done; these checks remain. Each needs a platform, a CI run or the owner.

- [ ] P.1 Owner review of `docs/spec/projection-format.md`.
- [ ] P.2 Golden, property and CLI suites (2.1, 3.3, 6.1) on macOS and Windows (CI jobs exist; blocked like
      harden-for-alpha 6.5).
- [ ] P.3 30-minute nightly parser fuzz and 30-minute soak (needs CI).
- [ ] P.4 `~/`, `..` and Windows drive paths with real files on macOS and Windows.
- [ ] P.5 Print and check + apply timings on the macOS and Windows reference machines; record in Results.
