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

## Implementation notes (deviations decided during apply)

- **Field registry instead of one struct per item type (D5).** Head fields the core reasons about (ids, `trackId`,
  `type`, `startTime`, `endTime`, track `order`) are typed; all other fields live in an ordered field map described
  by a declarative registry (`ove-core/src/schema.rs`). Validation, default omission, the JSON Schema, the schema
  reference document and (in change 3) the projection attribute order and `timeline.d.ts` are all generated from it,
  which keeps one source of truth with less code. Unknown fields keep their original order; known head fields are
  written in a canonical order first.
- **Four ops beyond the reference's ten.** `addMedia`, `updateMedia`, `removeMedia` (registry + media items) and
  `updateProject` (name and project metadata such as `cast`). The reference performs these outside its op list,
  but our rule is that `project.json` changes only through the pipeline (rename, media registration in change 3,
  probe results in change 5).
- **Lock holder info in `.lock.holder`, not `.lock` (D8).** Windows byte-range locks are mandatory, so another
  process could not read holder info inside the locked file. The advisory lock itself is on `.lock` (std
  `File::try_lock`: `flock` on macOS, `LockFileEx` on Windows).
- **Item map is a persistent ordered map** (`imbl::OrdMap`, MPL-2.0 unmodified) so a batch clone is O(1) (D6).
  Measured: in-memory apply on 5,000 items went from 2.2 ms to 72 µs (p50).
- **Prune cut point (D10).** Records are pruned up to the newest periodic snapshot at or before the oldest revision
  retention requires, so every retained revision stays reconstructable; retention may keep up to 199 extra records.
  (Found by the retention test: cutting exactly at the 2,000th record made the oldest retained revision unbuildable.)
- **Snapshot cadence.** The once-per-second `project.json` rewrite runs on the committing thread; on a 2.7 MB
  document it shows as a ~60 ms outlier once per second (p95 unaffected). Moving it to a background writer is a
  follow-up for the GUI (change 7).
- **`ove-doc undo/redo` across processes.** Session undo stacks are per process (D7). The test CLI's `undo` returns
  to the state before the latest revision and `redo` reverts the latest revision if it is an undo; both are restores
  recorded with source `undo`/`redo`.

## Results

Environment: Linux x86_64 dev container (4 vCPU, virtio disk, ext4-on-overlay), Rust 1.97.0, release build for
timings. macOS and Windows numbers come from the CI jobs and the reference machines (pending).

| Check | Result |
|---|---|
| Fixture round trip (every item type; unknown fields at every level) | byte-identical |
| Property test, random document + random batch (10,000 cases; ~40% of batches accepted, the rest rejected across 11 codes) | pass, 2.3 s |
| Property test, `apply(diff(a, b))` both directions; JSON round trip (1,000) | pass |
| Open-after-replay equals in-memory state, 1,000 random batch sequences incl. undo/redo | pass, ~60 s |
| Crash harness (task 4.3), child killed 1,000 times: 760 at failpoints inside record/snapshot/index write, flush, rename, dir-sync; 240 by external SIGKILL at random moments | every reopen = state before or after the last acknowledged batch |
| Faults: disk full (write, flush), permission denied, file busy (rename) | batch rejected with `disk.full` / `permission.denied` / `file.busy`, state intact, no temp files left |
| Second writer (real second process), stale lock after `abort()` | refused naming pid/process/host; takeover detected and logged |
| Open 5,000-item project (2.7 MB `project.json`), p50 of 10 | **116 ms** (budget 200 ms) |
| 1,000 single-op batches incl. durable record write | **p50 0.71 ms, p95 1.73 ms**, max 64 ms (snapshot second) (budget p95 10 ms) |
| Durable record write alone (`fdatasync` + rename + dir fsync) | p50 0.59 ms, p95 3.6 ms |
| 10,000-revision project | commit 0.96 ms each; open with replay 58 ms; rebuild an old revision 55 ms; prune 8,000 records 0.74 s |

| Fuzz, 3 minutes per target locally (libFuzzer, nightly toolchain): `read_json` 1.62 M runs, `apply_batch` 3.06 M runs | no crashes, no panics |

Not yet verified (tasks left open): owner review of `docs/spec/project-schema-v1.md` (1.1); the 30-minute nightly
fuzz runs (2.4, workflow added, short local runs below); every "on both OSes" check (4.1, 4.3, 4.5, 5.1, 5.2, 6.2)
until the macOS/Windows CI jobs have run; SMB-share library refusal on real network volumes (5.1, detection code is
per-OS: `MNT_LOCAL`, `GetDriveTypeW`/UNC, Linux `f_type`); reference-machine performance (6.1).
