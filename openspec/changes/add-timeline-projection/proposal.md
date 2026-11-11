# Proposal: add-timeline-projection

## Why

The product's defining feature is that people and agents edit the same timeline: the agent edits a readable text
file, the GUI edits the document, and neither loses the other's work. That requires a projection of the document to a
literal TSX file and a merge that applies file edits onto a document that may have moved on. It is the hardest
algorithm in the product, so it is specified and property-tested before any UI depends on it.

## What Changes

- Printer: deterministic `timeline.tsx` projection of the document, with a header (aspect, duration, counts, media
  table, cast, diagnostics, pending count, `rev`) and `timeline.d.ts` typings, rewritten after every revision.
- Parser and checker: parse-only TSX with literal attribute values, line/column diagnostics (errors and warnings),
  the tag and attribute mapping in docs/spec/document-model.md (Projection rules).
- Apply: diff the file against its base revision, rebase onto the current document with item-level 3-way merge,
  conflict policy (`file` wins by default, `--prefer editor`), conflict and removal reporting, as one new revision.
- Text-edit apply: `{oldText, newText}` edit lists against the printed file (the reference's `apply_timeline` contract),
  as a core function; MCP exposure is change 8.
- Media references by header name, id or path; new paths registered as media entries awaiting probe.
- Pending placeholder items: created, counted, filled in place keeping their id.
- Verify: lint registry with the structural lints (overlaps, gaps, clips past their source, missing/offline media,
  pending items); later changes register more lints.
- CLI commands working headless on bundles: `source print`, `source check`, `source apply`, `verify`, `history`,
  `undo`, with `--json` and the reference exit codes (0 ok, 1 refused).

Out of scope: `--eval` scripts (need the sandbox from change 4; added there), routing through the running app's control
socket (change 8), live apply in the open editor (change 7), lints that need rendering or text measurement
(off-screen text: change 11; invalid compositions: change 4), auto-reframe defaults (change 6).

References: docs/spec/document-model.md (Projection rules, Apply / merge, Verify), docs/spec/cli.md (`source`, `verify`,
`history`, `undo`), docs/spec/mcp-tools.md (`print_timeline`, `apply_timeline`, `verify_timeline`, `undo_timeline`).

## Capabilities

### New Capabilities
- `timeline-projection`: printing the document as TSX, checking and parsing projection files, merging file edits onto
  the live document, text-edit apply, media references in the projection, pending placeholders, verify lints, and the
  CLI commands that expose them.

### Modified Capabilities
- None (builds on `project-document` without changing its requirements).

## Impact

- New crate `ove-projection` (printer, parser, diff, merge, lints) and the first commands of the `ove` CLI binary.
- The projection format becomes a public contract for agents and templates; changes after this need compatibility
  rules.
