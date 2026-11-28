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

An edit to an item that the person deleted meanwhile is refused when `prefer` is left out: the call fails with
`edit.conflict`, nothing is applied, the message names each item (name and id), and `details.items` lists them. Ask
the person whether to bring the items back, and only then send the edit again with `prefer: "file"`. This rule is
advisory: the tool does not check that the person chose. `check_timeline` (the dry run) without `prefer` reports the
same `edit.conflict` refusal, so a check never passes an apply that would be refused. `ove source apply` and
`ove source check` keep "file wins" and re-create the items unless `--prefer editor` is given.

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

## Generation

`list_models` lists the generation models with their list prices, whether the person has added the provider's key,
and the session's spend and cap. `generate_image`, `generate_video`, `generate_voice`, `generate_music` and
`generate_sfx` spend the person's money, so they take two calls and a person:

1. Call without `confirm`. The receipt gives `plan`, `estimateUsd` and `quoteId`. Nothing is spent, and neither the
   project nor the provider is touched.
2. Call again with the same arguments and `confirm: "<quoteId>"`. A person then approves the price: on a price card
   in the running app, on the question card in the in-app assistant, or, when the app is not running, in the MCP
   client's confirmation prompt (elicitation). A client without one is refused with `confirm.required`.

The `confirm` argument alone never spends. Refusals: `confirm.required` (nobody could be asked),
`confirm.declined`, `quote.unknown`, `quote.expired` (quotes last 10 minutes), `quote.mismatch` (a quote is valid
only for the exact request it priced), `quote.used`, `spend.cap` (the session's approved spend would pass the
person's limit in Settings → Generation, $5 unless they change it), `spend.unknown` (a model without a known price)
and `key.missing`.

Approved, the receipt gives `jobId`, the placeholder's `itemId` and its printed `line`, and the new `rev`. The
placeholder is one revision of yours; the generation worker fills it as another revision of yours, so
`undo_my_changes` removes both. `generation_status` follows a job (`wait: true` waits up to two minutes),
`cancel_generation` stops it.

## Feedback

`send_feedback {category, severity, summary}` drafts a report about the app for the person. It is stored on this
computer only (`<config>/feedback/`) and never sent: the person reads it in Help → Feedback, edits or deletes it, and
copies it to send it themselves. Write the summary in your own words and leave out names, paths and what the person
typed. The app and OS versions, the session's last 15 tool names and its last error code are added. A session files
at most 8 reports (`feedback.limit`); an exact duplicate is not stored again.

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
| `measure_loudness` |  | mcp, pinned | read | `project`? `media`? `timeline`? |
| `look` |  | in-app, mcp, pinned | read | `project`? `at`? `every`? `count`? `width`? `stats`? `grid`? `storyboard`? `backend`? `view`? `scope`? |
| `screenshot` |  | mcp, pinned | read, app | `width`? |
| `export_video` |  | mcp, pinned | write | `project`? `out` `preset`? `format`? `quality`? `resolution`? `height`? `frameRate`? `fps`? `range`? `videoCodec`? `profile`? `bitDepth`? `alpha`? `videoBitrate`? `audioCodec`? `audioBitDepth`? `settings`? `audioOnly`? `loudness`? `wait`? `backend`? |
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
| `extract_audio` |  | mcp, pinned | write | `project`? `media` `format`? |
| `list_models` |  | in-app, mcp, pinned | read | `kind`? |
| `generate_image` |  | in-app, mcp, pinned | spends | `project`? `prompt` `model`? `aspect`? `image`? `durationSeconds`? `start`? `track`? `name`? `confirm`? |
| `generate_video` |  | in-app, mcp, pinned | spends | `project`? `prompt` `model`? `durationSeconds`? `aspect`? `image`? `negativePrompt`? `seed`? `start`? `track`? `name`? `confirm`? |
| `generate_voice` |  | in-app, mcp, pinned | spends | `project`? `text` `model`? `voice`? `speed`? `stability`? `style`? `start`? `track`? `name`? `confirm`? |
| `generate_music` |  | in-app, mcp, pinned | spends | `project`? `prompt` `model`? `durationSeconds`? `instrumental`? `start`? `track`? `name`? `confirm`? |
| `generate_sfx` |  | in-app, mcp, pinned | spends | `project`? `prompt` `model`? `durationSeconds`? `start`? `track`? `name`? `confirm`? |
| `generation_status` |  | in-app, mcp, pinned | write | `id` `project`? `wait`? `cancel`? |
| `cancel_generation` |  | in-app, mcp, pinned | destructive | `id` `project`? |
| `send_feedback` |  | in-app, mcp, pinned | write | `category` `severity` `summary` |
| `job_status` |  | mcp, pinned | read | `job_id` `wait`? |
| `cancel_job` |  | mcp, pinned | write | `job_id` |
| `list_export_jobs` |  | mcp, pinned | read | `all`? |
| `save_range_as_media` |  | mcp, pinned | write | `project`? `start` `end` `replace`? `tracks`? `name`? `small`? |
| `list_tasks` |  | in-app, mcp, pinned | read, app | `project`? `kind`? |
| `wait_tasks` |  | in-app, mcp, pinned | read, app | `project`? `kind`? `timeoutSeconds`? |
| `cancel_task` |  | in-app, mcp, pinned | destructive | `id` |
| `list_skills` |  | in-app, mcp, pinned | read |  |
| `read_skill` |  | in-app, mcp, pinned | read | `id` |
| `manage_skills` |  | in-app | destructive | `action` `id`? `text`? |
| `inspect_color` |  | in-app, mcp, pinned | read | `project`? `at`? `item`? `isolate`? `reference`? `referenceAt`? `strength`? `height`? `backend`? |
| `organize_media` |  | in-app, mcp, pinned | write | `project`? `ops` |
| `search_media` |  | in-app, mcp, pinned | read | `project`? `query`? `scope`? `media`? `limit`? `similarTo`? |
| `media_transcript` |  | in-app, mcp, pinned | read | `project`? `media` `from`? `to`? |
| `inspect_media` |  | in-app, mcp, pinned | read | `project`? `media` `overview`? `tileWidth`? |
| `sync_items` |  | in-app, mcp, pinned | write | `project`? `reference` `targets` `mode`? `window`? `minConfidence`? `drift`? `dryRun`? |
| `detect_beats` |  | in-app, mcp, pinned | read | `project`? `target` `from`? `to`? |
<!-- tools:end -->
