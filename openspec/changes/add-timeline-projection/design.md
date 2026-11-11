# Design: add-timeline-projection

## Context

See proposal.md (Why). Builds on `project-document` (ops, invariants, revisions, restore of any retained revision).
Contracts: docs/spec/document-model.md; requirements: specs/timeline-projection.

## Goals / Non-Goals

**Goals:**
- A merge whose behaviour can be stated as properties and proven by generated tests, not by examples alone.
- A projection format agents read and edit reliably: stable layout, small diffs, precise diagnostics.

**Non-Goals:**
- Executing any code from the projection (scripts are change 4's sandbox).
- Character-level merge of text fields (a text field is one value).

## Decisions

### D1. Parser: oxc, wrapped
Parse with `oxc_parser` in TSX mode, walk the AST into a projection model, and reject every non-literal node with its
span converted to line/column. All oxc usage lives in one module so a version bump touches one file; the oxc version is
pinned exactly. Alternatives: hand-written parser (full control, but JSX edge cases — entities, nested braces, string
escapes — would be re-invented); swc (heavier, slower compile); tree-sitter (good recovery, weaker exactness on
literal values).

### D2. Printer: hand-written, deterministic
Layout: header comment, one `<track>` element per track, one item per line when it fits in 120 columns, otherwise one
attribute per line; attributes in a fixed order per item type (identity, timing, media, content, look, sound, extra
fields sorted by key). Numbers printed as the shortest decimal that round-trips at microsecond precision. Strings
printed quoted when they contain no backslash, quote or newline, otherwise as a braced JSON string. Default-valued
fields omitted. Why: stable layout keeps agent edits and diffs small and makes golden tests meaningful. Alternative:
generic JS code generator — rejected, formatting would change with the generator's version.

### D3. Diff: base → file
The base document is reconstructed from the revision named by the header `rev` (project-document restore path). Both
base and file are normalised (defaults filled, times in microseconds) and compared per item id, per field, and one key
deeper for object fields; arrays compare as whole values. Output: a change set of track changes, item adds, field
sets, field unsets, item removals and aspect change.

### D4. Rebase onto current: the merge rule
For each changed field in the change set, compare base value B, file value F and current value C:
- C == B → take F.
- C == F → nothing to do (already applied; this makes re-applying the same file a no-op).
- otherwise → conflict: F under `prefer file` (default), C under `prefer editor`; reported either way.
Item added in file → added with a new id. Item removed in file: removed if it still exists; if C changed it since base,
it is a conflict — removed under `prefer file`, kept under `prefer editor`. Item edited in file but removed in current:
conflict; re-created with its id under `prefer file`, dropped under `prefer editor`. Track order: treated as one value
per the same rule. Items in a track are positioned by time, so no list-position merge is needed — the main reason
this merge stays simple and provable.
The merged result is applied as one project-document batch, so invariants are enforced and the whole apply is atomic;
an empty change set creates no revision.
Alternatives: CRDT merge (exact for collaboration, but our conflicts are rare and should be visible, not silently
resolved); line-based text 3-way merge (merges formatting, not meaning; breaks on reformatting).

### D5. Properties the test suite proves
Generated base documents B, a random file edit B→F and a random concurrent editor edit B→C:
1. Round trip: `parse(print(D)) == normalise(D)` and `print(parse(print(D))) == print(D)`.
2. Unchanged file: applying `print(B)` onto C yields C exactly and no revision.
3. File-only changes: when C == B, the result equals F.
4. Disjoint edits: when F and C touch disjoint (item, field) pairs, the result contains both edits and nothing else.
5. Conflicts: for every (item, field) changed on both sides to different values, the result holds the policy's value
   and the conflict is reported exactly once.
6. Idempotence: applying the same file twice — second apply is a no-op.
7. Validity: the result always satisfies document invariants, or apply is refused with nothing changed.

### D6. Text-edit apply
Edits are located by exact substring search in the projection printed at the current rev; each must match once; spans
must not overlap; replacements are applied from the end backwards; the result goes through the normal check + apply
with that rev as base. Whole-source replacement skips the edit step.

### D7. Media references
`src` resolution order: header media name → media id → path. Paths are expanded (`~`), resolved relative to the bundle,
canonicalised for comparison, and must exist. New files become media entries with status `awaiting-probe` (change 5
probes them); their media name is the file stem, de-duplicated with a numeric suffix.

### D8. Typings
`timeline.d.ts` is generated from the same type definitions as the document model (one source), so editors and agents
get completions that match what the parser accepts. CI checks it is regenerated.

### D9. CLI
`ove` binary (working name) built with clap; resolves a project by name, id or bundle path through the library; opens
the project through `ove-store` (taking the write lock for `apply`/`undo`); prints JSON on `--json`. If the write lock is
held by another process, the command is refused (exit 1) naming the holder; routing through the running app arrives in
change 8.

## Risks / Trade-offs

- [Field-level merge can produce a combination neither side intended (e.g. new start from file + new end from GUI makes
  a very short clip)] → invariants reject impossible states; verify flags suspicious results; conflicts are reported so
  the agent sees them. Accepted: same behaviour as the reference.
- [oxc API changes between versions] → exact pin, single wrapper module, parser golden corpus catches behaviour changes.
- [Reconstructing old base revisions costs time] → periodic snapshots (project-document D10) cap replay at 200 records;
  base reconstruction is cached per rev for the duration of a session.
- [Agents print, wait long, then apply against a pruned base] → clear refusal message instructing to print again;
  retention keeps ≥ 2,000 revisions.

## Migration Plan

None (new capability). The projection carries a format marker in its header so future layout changes stay readable.
