# OpenVideoEditor

Agent-native video editor for production houses. Planned as a Rust rebuild of a proven reference design, then
extended for production workflows.

## Docs

- `docs/01-reference-teardown.md` — how the reference app is built: architecture, every screen, main flows, weak spots.
- `docs/02-plan.md` — draft build plan (domain-neutral; production-house plan to follow).
- `docs/spec/` — rebuild contracts:
  - `document-model.md` — bundle layout, project document, items, projection rules, apply/merge ops, chat log
  - `ipc.md` — renderer↔main handlers and events, library worker, CLI↔app control socket
  - `cli.md` — CLI command surface (84 commands with arguments and flags)
  - `mcp-tools.md` — 50 MCP tools with parameters
  - `api.md` — 45 cloud endpoints (client-observed)
  - `agents.md` — agent providers, launch contracts, prompt rules, UI tools, templates
  - `presets.md` — effects (41) with parameter ranges, captions (21), layouts (15), transitions, LUTs, text animations,
    shortcuts (27 actions, 5 presets), fonts, generation models
  - `desktop-shell.md` — windowing/tabs, menus, document types, fuses, entitlements, updates, auth, plan gating

Status: research and specification. No code yet.
