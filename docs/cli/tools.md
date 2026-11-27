# Agent tools

Every tool an agent can call is defined once, in the tool registry (the `ove-tools` crate): its name, description,
input schema, annotations, and the surfaces it appears on. Three surfaces list tools from the registry:

- **in-app**: the assistant inside the app. It works on the project open in the editor, so `project` is optional.
- **mcp**: `ove mcp`, for agents that run outside the app (Claude Code, Codex and others).
- **pinned**: `ove mcp --project <p> [--turn <id>]`, bound to one project for a local agent session. Calls that name
  another project are refused with `project.pinned`. An unbound `ove mcp` session can set a default project with
  `use_project`.

A tool that appears on two surfaces has the same name, description and schema on both, and gives the same result for
the same input. The command line runs the same handlers: `ove source apply` runs `apply_timeline`, `ove media add`
runs `add_media`, and so on. The tool list is in a fixed order, so it does not change between requests.

## Results

A successful call returns a receipt: a JSON object (MCP `structuredContent`, `--json` output) and, for frame tools,
PNG images. A refused call returns `{"error": {"code", "message", "details"}}` (MCP `isError: true`).

An unknown tool name fails with code `tool.unknown`; `details.suggestions` lists the closest tool names. Over MCP this
is a JSON-RPC error (-32602) with the code and suggestions in `data`.

Arguments are checked before the tool runs:

- `argument.unknown`: a name the tool does not take.
- `argument.missing`: a required argument is absent (also `project` when the session has no project).
- `argument.invalid`: a value of the wrong type.

## Former names

The in-app assistant's former tool names still work for one release: `undo_last` is `undo_timeline`, `import_media`
is `add_media`, and `seek_viewer` is `seek` (its argument is now `seconds`).

## Edit with text edits

`apply_timeline` takes text edits on every surface. Print the timeline, then send only the changed text:

```json
{"project": "Demo", "rev": 1794433052235,
 "edits": [{"oldText": "name=\"Title\"", "newText": "name=\"Opening title\""}]}
```

Each `oldText` must occur exactly once in the printed timeline of `rev`. An `oldText` that does not occur is refused
with `edit.anchor`, and one that occurs more than once with `edit.ambiguous`; `details.index` names the edit. A whole
replacement file (`source`) is the second mode.

## Receipts

`apply_timeline` returns, besides the counts, conflicts and warnings:

- `rev`: the new revision. The next text edit can use it with the lines below, without printing again.
- `changedItems`: `{id, text}` for each added or changed item (at most 30; `moreChanged` counts the rest). `text` is
  the item's printed element, exactly as `print_timeline` prints it, so it can be the `oldText` of the next edit.
- `removedItems`: `{id, name}` for each removed item.
- `shifted`: one `{track, from, by, count}` rule for each group of 3 or more items on one track that moved by the
  same amount (in seconds); those items are not listed one by one.
- `trackOrderChanged`: whether the order of tracks changed.

Text edits may be made against an older `rev`: they are applied to the text printed for that revision and merged
onto the current document, so edits the person made meanwhile stay. Only a real conflict is reported in
`conflicts` (with `prefer: "editor"` the person's value is kept).

## Undo

Every revision records its actor: the kind (user, assistant, cli, mcp, template, generation), and the session, turn
and client name when known. Agents undo only their own changes:

- `undo_timeline` undoes the session's latest change that is not undone yet (or the session's revision `rev`).
- `undo_my_changes` undoes all of the session's changes (`scope: "session"`) or one turn's (`scope: "turn"`) as one
  revision.

Both keep every other change. When a later change by someone else touched the same fields, they refuse with
`undo.conflict` and list the items in `details.conflicts`. Naming a revision of another actor is refused with
`undo.notYours`. People can undo any session's changes from the command line with
`ove undo <project> --any --session <id>`.

## Frames for agents

- `look` and `grab_frame` take `grid: true`: the returned image carries a labelled grid in the units item transforms
  use (reference pixels from the canvas centre: x to the right, y down; lines every twelfth of the canvas width, so
  160 on a 1920-wide canvas), plus the title-safe (80 %) and action-safe (90 %) boxes. A `grab_frame` grid is in the
  frame's own pixels. The grid is drawn only on these returned frames, never in export.
- `filmstrip` and `look` take `storyboard: true`: about 120 frames across the whole file or timeline, repeats
  dropped, at most 36 tiles in 6 columns. Each tile has its timecode burned in; on timeline sheets also the ids of
  the items visible then. The receipt lists each tile's `at`, `timecode` and `items`.
- The in-app assistant has the frame and analysis tools too: `look`, `grab_frame`, `filmstrip`, `detect_scenes`,
  `audio_levels` and `detect_silence`. Their images reach it as image blocks.

## Examples

```sh
ove docs tools                                     # exit 0
ove source print Demo --json                       # exit 0
ove source apply Demo edited.tsx --json            # exit 0
ove undo Demo --json                               # exit 0
ove source print Nope                              # exit 1
```

## Tools

<!-- tools:begin -->
| Tool | Former names | Surfaces | Kind | Arguments |
|---|---|---|---|---|
| `list_projects` |  | mcp | read | `search`? |
| `create_project` |  | mcp | write | `name` `aspect`? `open`? |
| `show_project` |  | mcp, pinned | read | `project`? |
| `rename_project` |  | mcp, pinned | write | `project`? `name` |
| `print_timeline` |  | in-app, mcp, pinned | read | `project`? `from`? `lines`? |
| `check_timeline` |  | mcp, pinned | read | `project`? `source` `prefer`? |
| `apply_timeline` |  | in-app, mcp, pinned | write | `project`? `rev`? `edits`? `source`? `prefer`? |
| `verify_timeline` |  | in-app, mcp, pinned | read | `project`? |
| `timeline_history` |  | mcp, pinned | read | `project`? |
| `undo_timeline` | `undo_last` | in-app, mcp, pinned | destructive | `project`? `rev`? |
| `list_media` |  | in-app, mcp, pinned | read | `project`? `all`? |
| `add_media` | `import_media` | in-app, mcp, pinned | write | `project`? `paths` `track`? `at`? `noTimeline`? |
| `remove_media` |  | mcp, pinned | destructive | `project`? `media` |
| `relink_media` |  | mcp, pinned | write | `project`? `media`? `path`? `folder`? |
| `probe_media` |  | mcp, pinned | read | `project`? `media` |
| `grab_frame` |  | in-app, mcp, pinned | read | `project`? `media` `at`? `width`? `grid`? |
| `filmstrip` |  | in-app, mcp, pinned | read | `project`? `media` `count`? `every`? `from`? `to`? `columns`? `tileWidth`? `storyboard`? |
| `detect_scenes` |  | in-app, mcp, pinned | read | `project`? `media` `threshold`? |
| `audio_levels` |  | in-app, mcp, pinned | read | `project`? `media` `points`? |
| `waveform_image` |  | mcp, pinned | read | `project`? `media` `width`? `height`? |
| `detect_silence` |  | in-app, mcp, pinned | read | `project`? `media` `thresholdDb`? `minDuration`? `mode`? `minPause`? `padding`? `relativeDb`? |
| `measure_loudness` |  | mcp, pinned | read | `project`? `media` |
| `look` |  | in-app, mcp, pinned | read | `project`? `at`? `every`? `count`? `width`? `stats`? `grid`? `storyboard`? |
| `screenshot` |  | mcp, pinned | read, app | `width`? |
| `export_video` |  | mcp, pinned | write | `project`? `out` `height`? `fps`? |
| `export_timeline_file` |  | mcp, pinned | read | `project`? `file` `fps`? |
| `import_timeline_file` |  | mcp, pinned | write | `project`? `file` `fps`? |
| `app_context` |  | mcp, pinned | read, app |  |
| `open_in_app` |  | mcp, pinned | write, app | `project`? |
| `seek` | `seek_viewer` | in-app, mcp, pinned | write, app | `seconds` |
| `play` |  | mcp, pinned | write, app |  |
| `pause` |  | mcp, pinned | write, app |  |
| `docs` |  | mcp, pinned | read | `topic`? |
| `check_code` |  | in-app, mcp, pinned | read | `project`? `composition` `duration`? |
| `ask_user` |  | in-app | read | `question` `options`? |
| `undo_my_changes` |  | in-app, mcp, pinned | destructive | `project`? `scope`? `turn`? |
| `use_project` |  | mcp, pinned | read | `project` |
| `edit_timeline` |  | in-app, mcp, pinned | write | `project`? `request` `rev`? `preview`? |
| `list_models_local` |  | in-app, mcp, pinned | read |  |
| `get_transcript` |  | in-app, mcp, pinned | read | `project`? `clips`? `segments`? `offset`? `limit`? `model`? `language`? |
| `remove_words` |  | in-app, mcp, pinned | write | `project`? `ids`? `ranges`? `match`? `pause`? `apply`? `rev`? |
| `remove_silence` |  | in-app, mcp, pinned | write | `project`? `clips`? `minPause`? `padding`? `relativeDb`? `apply`? |
| `find_fillers` |  | in-app, mcp, pinned | read | `project`? `clips`? |
| `search_transcript` |  | in-app, mcp, pinned | read | `project`? `phrase` |
| `transcribe_media` |  | in-app, mcp, pinned | write | `project`? `media` `format`? `language`? `model`? `verbatim`? `force`? |
| `duck_audio` |  | in-app, mcp, pinned | write | `project`? `music` `under` `depth`? `attack`? `release`? `hold`? `source`? `mode`? `dryRun`? |
<!-- tools:end -->
