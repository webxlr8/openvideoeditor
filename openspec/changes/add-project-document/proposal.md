# Proposal: add-project-document

## Why

Every surface of the product (GUI, CLI, MCP, agents) edits the same project document. If the document can be lost,
corrupted or changed outside one pipeline, nothing built on top can be stable. The document core has no UI and no
dependency on change 1, so it is built in parallel with it and tested to a higher bar than anything else.

## What Changes

- Project bundle on disk: create, open, close, list, rename, duplicate, move to Trash/Recycle Bin, with the layout in
  docs/spec/document-model.md (bundle layout) minus the parts owned by later changes.
- Canonical project document (`project.json`): format id and version, project fields, tracks, items, media registry;
  versioned schema with forward migrations and preservation of unknown fields.
- Document operations: the 10 op kinds from docs/spec/document-model.md (`addTrack`, `updateTrack`, `removeTrack`,
  `reorderTracks`, `unsetTrackFields`, `addItem`, `updateItem`, `removeItems`, `unsetItemFields`, `setAspectRatio`),
  validated against document invariants and applied atomically as a batch.
- Revisions: every applied batch creates a new revision with a strictly increasing `rev`; revision history with
  restore; undo/redo where one batch is one step.
- Durable, crash-safe persistence and single-writer locking across processes.
- Media registry entries (ids, paths, ownership, online/offline state) — metadata only; import/probe is change 5.

Out of scope: the TSX projection and merge (change 3), media import/probe/proxies (change 5), chat logs (change 9),
templates folder (change 13), single-file project archive (change 7), UI screens (change 7).

Deliberate differences from the reference, with reasons:
- No `accounts/<hash>/` level in the library path: v1 has no accounts.
- Bundle extension is a working name (`.ove`) until the product is named (change 16).

## Capabilities

### New Capabilities
- `project-document`: project bundles and library, the canonical document and its schema, document operations and
  invariants, revisions, undo/redo, persistence durability and locking.

### Modified Capabilities
- None.

## Impact

- New crate `ove-core` (document model, ops, invariants, revisions; no I/O) and `ove-store` (bundle filesystem,
  persistence, locking, library).
- Defines the data contract every later change depends on; schema changes after this change require a migration.
