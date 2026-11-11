# OpenVideoEditor

Agent-native desktop video editor (macOS Apple silicon + Windows 11 x64), built in Rust with a Tauri shell.
v1 rebuilds a proven reference design feature for feature; later versions add production-house workflows.

## Docs

- `docs/03-architecture-plan.md` — decided architecture, alternatives, roadmap of OpenSpec changes, engineering
  standards, what the owner provides. Start here.
- `docs/01-reference-teardown.md` — how the reference app is built: architecture, every screen, main flows, weak spots.
- `docs/spec/` — behaviour contracts of the reference app that v1 rebuilds:
  - `document-model.md` — bundle layout, project document, items, projection rules, apply/merge ops, chat log
  - `ipc.md` — renderer↔main handlers and events, library worker, CLI↔app control socket
  - `cli.md` — CLI command surface (84 commands with arguments and flags)
  - `mcp-tools.md` — 50 MCP tools with parameters
  - `api.md` — 45 cloud endpoints (client-observed; not used in v1)
  - `agents.md` — agent providers, launch contracts, prompt rules, UI tools, templates
  - `presets.md` — effects (41) with parameter ranges, captions (21), layouts (15), transitions, LUTs, text animations,
    shortcuts (27 actions, 5 presets), fonts, generation models
  - `desktop-shell.md` — windowing/tabs, menus, document types, fuses, entitlements, updates, auth, plan gating

## Planning (OpenSpec)

Spec-driven with [OpenSpec](https://github.com/Fission-AI/OpenSpec) 1.17. Project rules: `openspec/config.yaml`.

```
npm install -g @fission-ai/openspec@1.17.0
openspec list                 # changes in flight and task progress
openspec show <change>        # proposal, specs, design, tasks
openspec validate --all --strict
```

Fully specified, in order: `add-desktop-shell-and-native-preview` (viewer gate R1), `add-project-document`,
`add-timeline-projection`. Changes 4–16 are listed in the roadmap and get specified just before they start.

Status: planning complete for changes 1–3. No code yet.
