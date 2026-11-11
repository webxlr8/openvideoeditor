# MCP server: `ove mcp`

`ove mcp` serves the `ove` commands as Model Context Protocol tools over stdin/stdout (JSON-RPC 2.0, one message
per line; protocol versions 2025-06-18, 2025-03-26 and 2024-11-05). Each tool runs the matching command in-process
with `--json`; the result is its JSON object (also as `structuredContent`), and refusals come back with
`isError: true` and `{"error": {"code", "message", "details"}}`. Image tools (frames, filmstrips, waveforms, looks,
screenshots) also return the PNGs as image content. Edits are recorded with source `mcp`.

Set up an agent:

```sh
ove skills install                                   # writes the openvideoeditor skill for Claude Code / Codex
claude mcp add openvideoeditor -- ove mcp            # Claude Code
codex mcp add openvideoeditor -- ove mcp             # Codex
```

Pass `--library <folder>` before `mcp` to serve another library. Tools:

| Tool | Command |
|---|---|
| list_projects, create_project, show_project, rename_project | `project list / create / show / rename` |
| print_timeline, check_timeline, apply_timeline | `source print / check / apply` (the edited text is passed as `source`) |
| verify_timeline, timeline_history, undo_timeline | `verify`, `history`, `undo` |
| list_media, add_media, remove_media, relink_media, probe_media | `media list / add / remove / relink / probe` |
| grab_frame, filmstrip, detect_scenes | `media grab / filmstrip / scenes` |
| audio_levels, waveform_image, detect_silence, measure_loudness | `audio levels / waveform / silence / loudness` |
| look, screenshot | `look`, `screenshot` |
| export_video, export_timeline_file, import_timeline_file | `export`, `timeline export / import` |
| app_context, open_in_app, seek, play, pause | `app context / open / seek / play / pause` |
| docs | `docs` (`agents` is the editing guide) |
