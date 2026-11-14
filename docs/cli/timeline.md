# `ove` timeline commands

Working name `ove`. These commands work headless on project bundles in the library (`--library <dir>`, else
`OVE_LIBRARY`, else the platform default). `<project>` is a project name, id or bundle path.

Output: results on stdout, errors on stderr. With `--json`, stdout is exactly one JSON object and errors are
`{"error": {"code", "message", "details"}}` on stderr.

Exit codes: `0` ok, `1` refused (nothing changed), `2` usage error.

While another process has the project open for writing, `source apply` and `undo` are refused (exit 1) with
`project is open in <process> (pid N) on <host>`. Routing through the running app arrives with the control socket.

## Commands

| Command | Does |
|---|---|
| `ove source print <project> [--from <line>] [--lines <n>]` | Print the projection of the current revision (`timeline.tsx` content). |
| `ove source check <project> [file] [--prefer file\|editor]` | Parse, validate and plan an apply without changing anything. `file` defaults to the bundle's `timeline.tsx`; `-` reads stdin. |
| `ove source apply <project> [file] [--prefer file\|editor]` | Merge the file onto the current document as one revision (none if nothing changed). |
| `ove verify <project>` | Lints: overlaps, gaps on the main track, clips past their source, missing/offline media, pending items. Exit 1 when any finding is an error. |
| `ove history <project>` | Revisions, oldest first: rev, source, summary. |
| `ove undo <project> [rev]` | Revert the latest revision, or return to revision `rev`, as a new revision (source `undo`). |

## JSON shapes

`source apply --json`:

```json
{"baseRev": 1762000000000, "added": 1, "updated": 2, "removed": 1,
 "removedItems": [{"id": "i_3", "name": "Old take"}],
 "conflicts": [{"id": "i_1", "field": "endTime", "fileValue": 3, "editorValue": 3.5, "kept": "file", "message": "…"}],
 "warnings": [{"severity": "warning", "line": 31, "column": 22, "code": "object.key.unknown", "message": "…"}],
 "rev": 1762000000123, "changed": true}
```

`source check --json`: the same without `rev`/`changed`, plus `"ok": true`. A refused check or apply exits 1 with
`details.diagnostics` listing every problem with `severity`, `line`, `column`, `code` and `message`.

`verify --json`: `{"rev", "count", "findings": [{"lint", "severity", "itemIds", "message"}]}`.

`history --json`: `{"rev", "revisions": [{"rev", "parent", "time", "source", "summary", "opCount"}]}`.

`undo --json`: `{"rev", "changed", "target"}`.

## Refusal codes

| Code | Meaning |
|---|---|
| `check.failed` | The projection has errors (see `details.diagnostics`) or the result would be invalid. |
| `rev.missing` / `rev.malformed` | The header has no usable `rev:` line. Print the timeline again. |
| `rev.newer` | The header names a revision newer than the project's. |
| `rev.pruned` | The base revision is no longer retained. Print the timeline again. |
| `project.locked` | Another process has the project open for writing. |
| `verify.failed` | Verify found at least one error. |

## Examples

These run in the integration tests (`crates/ove-cli/tests/docs.rs`) against a library holding a project `Demo`
with one pending placeholder, and a file `edited.tsx` that renames one clip.

```sh
ove source print Demo --from 1 --lines 5            # exit 0
ove source print Demo --json                        # exit 0
ove source check Demo                               # exit 0
ove source check Demo edited.tsx --json             # exit 0
ove source apply Demo edited.tsx                    # exit 0
ove source apply Demo edited.tsx --json             # exit 0
ove verify Demo                                     # exit 1
ove history Demo --json                             # exit 0
ove undo Demo                                       # exit 0
ove source apply Demo broken.tsx                    # exit 1
ove source apply Demo edited.tsx --prefer editor    # exit 0
ove frobnicate                                      # exit 2
```
