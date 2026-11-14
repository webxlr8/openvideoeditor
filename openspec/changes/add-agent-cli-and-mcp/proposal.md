# Proposal: add-agent-cli-and-mcp

## Why

Agents (Claude Code, Codex, the in-app assistant) edit projects through the same tools a person can run in a
terminal. Change 3 gave the timeline commands on bundles; agents also need projects, media, inspection of footage and
audio, rendered looks at the timeline, and a way to reach the running app — the app holds the project's write lock, so
without a link the command line cannot edit what the user is looking at.

## What Changes

- `ove` command tree: `project` (list, create, show, rename, duplicate, remove, open, reveal), `media` (list, add,
  remove, relink, probe, grab, filmstrip, scenes), `audio` (levels, waveform, silence, loudness), `look`,
  `screenshot`, `app` (context, open, seek, play, pause), `docs`, `doctor`, `skills install`, `mcp`, next to the
  existing `source`, `verify`, `history`, `undo`, `export`, `timeline`.
- `ove-analyze`: frame grabs, contact sheets, scene cuts, levels, silence, ITU-R BS.1770 / EBU R128 loudness and
  composited timeline frames (through the export compositor).
- Control link (`ove-control`): the running app listens on a loopback port with a per-launch token published in
  `<library>/.ove-control.json` (owner-only). Methods: context, open, seek, play, pause, apply, restore.
- Edits while the app holds the project are sent to it with the revision they were computed from and applied as
  revisions of the app's session (source `cli` / `mcp`); stale requests are refused with `rev.changed` and retried.
- MCP server (`ove mcp`): stdio JSON-RPC, tools mirror the commands in-process; image tools return PNG content.
- Agent setup: `ove skills install` writes a skill for Claude Code / Codex; docs pages per command group embedded in
  the binary (`ove docs`); installers ship `ove` as a sidecar.

Out of scope (later changes): stock footage / music / SFX search and downloads, web capture, voice and generation
commands (change 14: bring-your-own provider keys), audio processing (stem separation, de-echo), mattes and cutouts,
transcription and captions commands (change 10), auth/update commands (change 16), launching Claude Code / Codex from
the app with project context (change 9).

## Impact

New crates `ove-control`, `ove-analyze`; `ove-cli` restructured into modules; desktop gains `control.rs`, a
`report_ui_state` command and a `RemoteOpened` event; `tools/package.sh` bundles the CLI.
