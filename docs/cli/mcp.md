# MCP server: `ove mcp`

`ove mcp` serves the agent tool registry (see `ove docs tools`) as Model Context Protocol tools over stdin/stdout
(JSON-RPC 2.0, one message per line; protocol versions 2025-06-18, 2025-03-26 and 2024-11-05). Each tool runs the
same handler as the matching `ove` command; the result is its JSON object (as `structuredContent`; the text content
is the same result as the in-app assistant gets), and refusals come back with `isError: true` and
`{"error": {"code", "message", "details"}}`. Image tools (frames, filmstrips, waveforms, looks, screenshots) return
the PNGs as image content. Edits are recorded with source `mcp`. `apply_timeline` takes text edits with the printed
`rev` (or a whole replacement `source`), so an agent does not resend the file.

Set up an agent:

```sh
ove skills install                                   # writes the openvideoeditor skill for Claude Code / Codex
claude mcp add openvideoeditor -- ove mcp            # Claude Code
codex mcp add openvideoeditor -- ove mcp             # Codex
```

Pass `--library <folder>` before `mcp` to serve another library.

`ove mcp --project <project> [--turn <id>]` starts a session bound to one project (the pinned server for local
agents): every project tool works on it without naming it, and a call that names another project is refused with
`project.pinned`. `--turn` records a turn id on every revision the session makes, so `undo_my_changes` with scope
`turn` can undo just that turn. Without `--project`, `use_project` sets a default project for the session. Each
`ove mcp` process is one session; its revisions record the session and the client name the client gives in
`initialize`, and agents undo only their own session's changes.

Tools:

| Tool | Command |
|---|---|
| list_projects, create_project, show_project, rename_project | `project list / create / show / rename` |
| print_timeline, check_timeline, apply_timeline | `source print / check / apply` (`apply_timeline` takes `edits` and `rev`, or the edited text as `source`) |
| verify_timeline, timeline_history, undo_timeline | `verify`, `history`, `undo` |
| list_media, add_media, remove_media, relink_media, probe_media | `media list / add / remove / relink / probe` |
| grab_frame, filmstrip, detect_scenes | `media grab / filmstrip / scenes` |
| audio_levels, waveform_image, detect_silence, measure_loudness, duck_audio | `audio levels / waveform / silence / loudness / duck` |
| look, screenshot | `look`, `screenshot` |
| export_video, export_timeline_file, import_timeline_file | `export`, `timeline export / import` |
| app_context, open_in_app, seek, play, pause | `app context / open / seek / play / pause` |
| docs | `docs` (`agents` is the editing guide) |
| check_code | (none; checks a motion-graphics code item) |
| list_models_local | `models list` (read-only; installing a model over 50 MB needs the person's consent) |
