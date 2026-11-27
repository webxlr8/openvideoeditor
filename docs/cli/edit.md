# `ove edit`: edit verbs

`ove edit <verb> <project> [options]` runs one edit verb on the timeline as one revision (one undo step). The verbs
are the same planners the editor and the assistant use, so every writer gets the same result:

- Requested times snap to the timeline's frame grid (`fps` on `<timeline>`, default 30).
- A ripple moves later items on the target tracks, on every sync-locked track (`syncLock`, on by default) and on
  every track that holds a link partner of a moved item.
- Linked items (the same `link` value) move, trim, split, slip and delete together. `--unlinked` edits only the
  named item (J and L cuts).
- Nothing on a locked track changes; a ripple that would move or cut an item there is refused (`ripple.blocked`).
- An edit never creates an overlap that was not there before, unless `--crossfade` allows it (trim, move).
- Edits stop at the source media's first and last frame; the plan lists the items that stopped (`limited`).

Times are seconds. `--dry-run` prints the plan and changes nothing. `--rev <rev>` names the revision the request was
made against: its times are moved through the ripples made since, so a request made before someone else's ripple
lands where it was meant. A revision newer than the project's is refused (`rev.newer`).
`--markers` makes timeline markers follow a ripple: markers after the edit shift with it, a point marker inside a
removed range is removed and a range marker that overlaps it shrinks. Without it, markers stay where they are.

## Verbs

| Verb | Options | Does |
|---|---|---|
| `split` | `--at`, `--item`/`--items` or `--tracks` | Cut the items (and their link partners) at `--at`, or every item under `--at` on the tracks (all unlocked tracks when none are given). |
| `trim` | `--item`, `--edge start\|end`, `--to` | Move one edge; later items stay. |
| `ripple-trim` | `--item`, `--edge start\|end`, `--to` | Move one edge and ripple: later items move by the length added or removed. A start edge keeps the item's start. |
| `ripple-delete` | `--items`, or `--tracks` and `--ranges A-B,C-D` | Remove items and close the space they used, or remove time ranges. |
| `delete` | `--items` | Remove items and their link partners, leaving the space empty. |
| `insert` | `--track`, `--at`, `--items-json` | Place new items at `--at` and push later items later. Item JSON gives `type`, fields and timing relative to the first item; `trackId` defaults to `--track`. |
| `overwrite` | `--track`, `--at`, `--items-json` | Place new items at `--at`, replacing what was there. |
| `lift` | `--tracks`, `--range A-B` | Remove a range, leaving a gap. |
| `extract` | `--tracks`, `--range A-B` | Remove a range and close it. |
| `close-gap` | `--track`, `--at` | Close the empty space at `--at`. Refused when a sync-locked track has something in that space. |
| `slip` | `--item`, `--delta` | Show other source frames in the same timeline span (`--delta` seconds later in the source). |
| `slide` | `--item`, `--delta` | Move an item between its neighbours, which grow and shrink. |
| `roll` | `--left`, `--right`, `--to` | Move the edit point between two adjacent items. |
| `move` | `--item`, `--start`, `[--track]` | Move an item (and its link partners) keeping its length. |
| `request` | `--request '<json>'` | The whole request as JSON, as MCP's `edit_timeline` sends it, for example `{"op":"rippleDelete","tracks":["a1"],"ranges":[{"start":12.3,"end":14}]}`. |
| `keyframes` | `--item`, `--property`, `--set T:V[:easing],…` or `--add …` or `--remove T,…`, `[--easing]` | Set, add or remove the keyframes of one property (x, y, scale, rotation, opacity, gaussianBlur, borderRadius, volume, crop.top, crop.right, crop.bottom, crop.left). Times are seconds from the item's start; each keyframe may name the easing of the segment that starts at it (linear, ease-in, ease-out, ease-in-out, hold). Same as MCP's `edit_keyframes`. |

## JSON shape

`--json` prints `{"verb", "request", "baseRev", "summary", "touched", "limited", "ops", "timeMap", "dryRun", "rev"}`.
`ops` are the document ops of the revision; `timeMap` (ripples only) lists per track the points where time moved.

## Refusal codes

| Code | Meaning |
|---|---|
| `item.missing` / `track.missing` | An id in the request does not exist (`details.culprits`). |
| `track.locked` | The item or the range is on a locked track. |
| `ripple.blocked` | The ripple would move or cut items on a locked track (`details.culprits`), or close a gap that is not empty on a sync-locked track. |
| `overlap.created` | The edit would make two items overlap; pass `--crossfade` to allow it (trim, move). |
| `edit.outside` | Nothing to split at that time. |
| `gap.none` | There is no gap at that time. |
| `slip.noSource` | The item has no source media to slip. |
| `roll.notAdjacent` | The two items do not meet at an edit point. |
| `range.empty` / `range.overlap` / `request.invalid` | The request is malformed. |
| `rev.newer` | `--rev` is newer than the project's revision. |
| `keyframe.outOfRange` | A keyframe time is before the item's start or after its end (`keyframes`). |
| `keyframe.property`, `keyframe.easing`, `keyframe.missing` | Unknown property or easing, or no keyframe at a time to remove (`keyframes`). |

## Examples

The project `Demo` has `a` [0, 4), `b` [4, 8) and `c` [8, 12) on track `v1`, `b`'s linked sound `bs` on `a1`, a music
bed on `music` with sync lock off, and a title `t` [10, 12) on `titles`.

```sh
ove edit split Demo --item a --at 2 --dry-run                               # exit 0
ove edit ripple-delete Demo --track a1 --ranges 5.0-6.0 --dry-run --json    # exit 0
ove edit ripple-delete Demo --items b --json                                # exit 0
ove edit trim Demo --item c --edge end --to 7                               # exit 0
ove edit slide Demo --item c --delta -0.5                                   # exit 0
ove edit move Demo --item a --start 20 --json                               # exit 0
ove edit close-gap Demo --track v1 --at 10                                  # exit 1
ove edit split Demo --item nope --at 1                                      # exit 1
ove edit keyframes Demo --item a --property opacity --set 0:1,2:0:hold    # exit 0
ove edit keyframes Demo --item a --property opacity --add 9:0              # exit 1
ove edit frobnicate Demo                                                    # exit 2
```
