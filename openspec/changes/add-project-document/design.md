# Design: add-project-document

## Context

See proposal.md (Why). Contracts come from docs/spec/document-model.md; requirements from specs/project-document.
This change has no UI; everything is exercised through library APIs, a small test CLI and test harnesses.

## Goals / Non-Goals

**Goals:**
- A document core that cannot lose or corrupt data under crashes, kills, full disks and concurrent processes.
- Fast enough that the GUI can apply an op per user gesture and agents can apply large batches.

**Non-Goals:**
- Multi-user collaboration or sync (not in v1).
- The text projection and 3-way merge (change 3 builds on the op/revision model defined here).

## Decisions

### D1. Crates
`ove-core`: types, ops, invariants, inverse computation, revision records, migrations — pure, no filesystem.
`ove-store`: bundles, library, persistence, locking, recovery. Alternative: one crate — rejected so the core can be
property-tested and fuzzed without I/O and reused by the CLI and agents unchanged.

### D2. Persistence = write-ahead revision log + snapshot
Each batch is committed by writing one revision record `.revs/<rev>.json` (forward ops, inverse ops, parent rev, time,
source, summary) to a temp file, flushing, renaming into place and flushing the directory. Only then is the batch
reported as applied. `project.json` is the canonical snapshot, rewritten atomically (temp file + flush + rename)
at most once per second while edits continue, and always on close. Open = read `project.json`, then replay every
record with `rev` greater than the snapshot's `stateUpdatedAt`, in order.
Why: a 5,000-item snapshot is several MB; rewriting it per gesture would miss the 10 ms budget. The log makes each
commit small and keeps `project.json` always a complete valid document.
Flush primitive: `F_BARRIERFSYNC` on macOS (orders writes so a crash yields old-or-new state, without the cost of
`F_FULLFSYNC`), `FlushFileBuffers` on Windows; `F_FULLFSYNC` on close and before pruning.
Alternatives: full snapshot per batch (too slow at size); SQLite for history (robust, but a second storage format to
recover and the reference contract is `.revs/<rev>.json`); CRDT document (Loro/Automerge: solves collaboration we do
not need, makes the JSON contract indirect).

### D3. rev values
`rev = max(now_ms, previous_rev + 1)`, matching the reference's use of `stateUpdatedAt` in milliseconds while staying
strictly increasing under clock skew or bursts.

### D4. Time representation
Wire format keeps the reference contract (seconds as JSON numbers). Inside the core, times are integer microseconds
(`i64`), converted on read and written with at most 6 decimals, so repeated edits cannot accumulate float drift and
equality checks in merge (change 3) are exact. Frame snapping is the editor's job, not the document's.

### D5. Model types and unknown fields
Typed structs per track/item type with a flattened `extra` map that captures unknown fields and writes them back in
the original key order. A JSON Schema is generated from the types and committed as `docs/spec/project.schema.json`;
CI fails if it drifts. Alternative: untyped JSON everywhere — rejected, invariants and migrations become unreviewable.

### D6. Applying batches
The document is held as persistent shared structures (items in an id-indexed map with `Arc` values). A batch is applied
to a cheap clone; invariants are checked on touched entities plus the global checks (unique ids, contiguous track
order); on success the clone replaces the current document, otherwise it is dropped. Inverse ops are computed from the
before-values during apply. Alternative: apply in place with rollback — rejected, harder to prove all-or-nothing.

### D7. Undo/redo
Session-scoped stacks of revision ids. Undo applies the stored inverse ops as a new batch (source `undo`); redo
re-applies the forward ops (source `redo`). Undo across app restarts is served by revision restore. Alternative:
persistent undo stacks — deferred; the reference does not show cross-session undo.

### D8. Locking
`<bundle>/.lock` held with an OS advisory lock (`flock` on macOS, `LockFileEx` on Windows) for as long as the project is
open for writing, containing pid, process name, start time and host. The OS releases the lock when a process dies,
so stale detection does not depend on pid reuse heuristics. Read-only opens take no lock.

### D9. Library listing
A derived `cache/index.json` (id, name, times, aspect, duration, item count, rev) is written with each snapshot.
Listing reads it; when it is missing or its rev is older than the newest revision record, the full document is
parsed and the index rewritten. Damaged bundles are reported, never fatal to the listing.

### D10. Retention
Revision records are kept for the last 2,000 revisions or 30 days, whichever keeps more; a full snapshot is written to
`.revs/<rev>.snap.json` every 200 revisions so restore never replays more than 200 records. Pruning runs on close.

### D11. Platform specifics
Trash via the platform API (NSFileManager `trashItem`, `IFileOperation` with recycle flag). Network volume detection:
`statfs` without `MNT_LOCAL` on macOS; `GetDriveTypeW == DRIVE_REMOTE` or a UNC path on Windows. Paths are stored as
given and normalised for comparison only (case-insensitive on Windows and on case-insensitive APFS volumes).

## Risks / Trade-offs

- [Snapshot lags the log by up to 1 s; an external tool reading `project.json` may see an older state] → the spec says
  only the app writes it and tools go through the core/CLI; the snapshot is rewritten before any CLI read that bypasses
  the core is possible (CLI uses the core in change 3).
- [Many small files in `.revs/`] → retention + periodic snapshots; measured on a 10,000-revision project.
- [`F_BARRIERFSYNC` does not guarantee durability of the very last batch on power loss] → accepted: the spec requires
  old-or-new state, which ordering guarantees; `F_FULLFSYNC` on close.
- [Antivirus on Windows holding files and failing renames] → retry rename with backoff up to 2 s, then surface an error;
  tested with a file held open by another process.
- [Schema mistakes discovered late] → schema reviewed against docs/spec/document-model.md item table before any code
  depends on it (task 1.1); migrations from day one.

## Migration Plan

First version of the format (`version: 1`). The migration framework and fixtures exist from this change so the first
real migration is routine.
