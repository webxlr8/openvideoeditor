# Agent tools

Every tool an agent can call is defined once, in the tool registry (the `ove-tools` crate): its name, description,
input schema, annotations, and the surfaces it appears on. Three surfaces list tools from the registry:

- **in-app**: the assistant inside the app. It works on the project open in the editor, so `project` is optional.
- **mcp**: `ove mcp`, for agents that run outside the app (Claude Code, Codex and others).
- **pinned**: `ove mcp` bound to one project for a local agent session.

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
| `grab_frame` |  | mcp, pinned | read | `project`? `media` `at`? `width`? |
| `filmstrip` |  | mcp, pinned | read | `project`? `media` `count`? `every`? `from`? `to`? `columns`? `tileWidth`? |
| `detect_scenes` |  | mcp, pinned | read | `project`? `media` `threshold`? |
| `audio_levels` |  | mcp, pinned | read | `project`? `media` `points`? |
| `waveform_image` |  | mcp, pinned | read | `project`? `media` `width`? `height`? |
| `detect_silence` |  | mcp, pinned | read | `project`? `media` `thresholdDb`? `minDuration`? |
| `measure_loudness` |  | mcp, pinned | read | `project`? `media` |
| `look` |  | mcp, pinned | read | `project`? `at`? `every`? `count`? `width`? `stats`? |
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
<!-- tools:end -->
