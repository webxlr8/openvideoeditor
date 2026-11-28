# `ove version`: named versions

A named version pins one revision of a project under a name, such as "client v2", with an optional note. History
pruning (after 2,000 revisions and 30 days) never removes a version: its snapshot lives in `.revs/versions/`, and
items that did not change between versions are stored once.

| Command | Does |
|---|---|
| `ove version save <project> <name> [--note <text>]` | Save the current revision as a version. A name in use is refused (`version.exists`). |
| `ove version list <project>` | The versions, oldest first: name, revision, time, who saved it, note, bytes added. |
| `ove version print <project> <name> [--from N] [--lines N]` | The version's timeline as its TSX projection (read-only). |
| `ove version diff <project> <from> [<to>]` | The items and markers added, removed, moved, trimmed or changed from one version to another, or to the current timeline when `<to>` is left out. |
| `ove version restore <project> <name>` | Make the timeline equal to the version, as one new revision; `ove undo` brings back what was there. |

MCP and the in-app assistant have the same commands as tools: `save_version`, `list_versions`, `print_version`,
`diff_versions` and `restore_version`.

## JSON shape

`ove version diff --json` prints `{"from", "to", "changes": [{"entity", "id", "name", "class", "before", "after",
"fields"}]}`. `entity` is `item` or `marker`; `class` is `added`, `removed`, `moved` (same length, new start or track),
`trimmed` (new length) or `changed` (same timing, other fields: listed in `fields`); `before` and `after` give
`{start, end, track}` in seconds.

## Refusal codes

| Code | Meaning |
|---|---|
| `version.exists` | A version with that name already exists. |
| `version.missing` | There is no version with that name. |
| `version.name` | The name is empty or longer than 120 characters. |

## Examples

The project `Demo` has a text item `a` from 0 to 4 s and `b` from 4 to 8 s; between the commands below, `a` is
trimmed to end at 3 s.

```sh
ove version save Demo v1 --note "first cut"     # exit 0
ove version save Demo v1                         # exit 1
ove version list Demo --json                     # exit 0
ove version diff Demo v1 --json                  # exit 0
ove version print Demo v1 --lines 3              # exit 0
ove version restore Demo v1                      # exit 0
ove version restore Demo nope                    # exit 1
```
