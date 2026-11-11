# Tasks: add-agent-cli-and-mcp

## 1. Control link

- [x] 1.1 `ove-control`: loopback server, token, private endpoint file, client; verify request/reply, wrong token,
      cleanup on drop (tests/link.rs).
- [x] 1.2 Desktop: serve context, open, seek, play, pause, apply, restore; UI reports selection and viewer mode and
      follows `RemoteOpened`; verify in the running app (Linux) that a CLI edit appears in the editor and undoes.
- [ ] 1.3 Verify the link on macOS and Windows builds (endpoint file permissions on Windows: library ACL).

## 2. Command tree

- [x] 2.1 Restructure `ove-cli` into modules with a shared `Ctx` and `Writer` (local or through the app) and retry on
      `rev.changed`; verify with a stand-in app holding the lock (tests/agent.rs).
- [x] 2.2 `project`, `media`, `app`, `docs`, `doctor`, `skills install`; docs pages with executed examples.
- [x] 2.3 `ove-analyze` and `media grab|filmstrip|scenes`, `audio levels|waveform|silence|loudness`, `look`,
      `screenshot`; verify on synthetic media (tests/media.rs) and loudness against FFmpeg `ebur128`.

## 3. MCP and agent setup

- [x] 3.1 `ove mcp` with the tool table; verify initialize, tools/list, edits, refusals and image results.
- [x] 3.2 `ove skills install` (Claude Code / Codex skills folders) and the agents guide (`ove docs agents`).
- [x] 3.3 Ship `ove` as an installer sidecar (tools/package.sh).
- [ ] 3.4 "Install command line tool" in Settings (symlink into PATH on macOS; PATH entry on Windows) — change 13.
- [ ] 3.5 Launch Claude Code / Codex from the app in the project folder with the skill and MCP preconfigured — change 9.
