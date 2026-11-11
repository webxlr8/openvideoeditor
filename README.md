# OpenVideoEditor

Agent-native desktop video editor (macOS Apple silicon + Windows 11 x64), built in Rust with a Tauri shell.
v1 rebuilds a proven reference design feature for feature; later versions add production-house workflows.

## Using the app

Install: `tools/package.sh` builds an unsigned installer for the machine it runs on (macOS `.dmg`, Windows `.msi` /
setup `.exe`, Linux `.deb`). For development: `cd apps/desktop && pnpm install && pnpm tauri dev`.

- **Home / Projects**: your projects with thumbnails (library: `~/Movies/OpenVideoEditor` on macOS,
  `%USERPROFILE%\Videos\OpenVideoEditor` on Windows, `OVE_LIBRARY` to override). Start a blank project, drop footage,
  or describe a video in the prompt box (the assistant starts on it).
- **Editor**: import media (probed, thumbnailed, appended to Main / Audio); the viewer plays the composited timeline
  (or one clip's source with the Source toggle); select, drag between tracks, trim edges with snapping, split (S),
  duplicate (D), copy/paste, delete, undo/redo, zoom; right-click menus; `?` lists shortcuts.
- **Inspector** for the selected clip: speed/reverse, transform, opacity, blend, crop, transitions; colour; volume and
  fades; text style; caption segments.
- **Panels**: Text (title styles), Captions (manual for now), Transitions, Effects (colour looks), Audio, Layout.
- **Assistant**: bring your own Anthropic API key (stored in the OS credential store). It reads and edits the
  timeline through `timeline.tsx`; each edit is one undo step; chats are saved in the project.
- **Export**: MP4 (H.264 + AAC) at 720p / 1080p / 4K, 24–60 fps, with progress and cancel; `ove export` does the same
  from the command line.
- Every edit is a durable revision; `ove source print|apply` and agents see the same timeline.
- Not yet: automatic transcription, motion-graphics code items, effects/LUT shaders beyond colour grading, templates,
  AI generation, interchange (FCPXML/EDL), signed installers and auto-update.

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

Status: changes 1–3 implemented, plus the render/export engine, editor workspace and assistant (v1 slices of
changes 6–9) (see each change's `design.md` Results for what is verified and what still needs the
macOS / Windows machines). Layout:

```
crates/ove-core        document model, ops, invariants, revisions          crates/ove-store   bundles, durable log, locking, library
crates/ove-projection  timeline.tsx printer/parser/merge/verify            crates/ove-cli     `ove` command line
crates/ove-media       probe + decode (FFmpeg)                             crates/ove-playback clock, transport, A/V sync
crates/ove-gpu         wgpu viewer renderer                                crates/ove-bench   headless playback benchmark
apps/desktop           Tauri 2 shell + React UI                            tools/ffmpeg       LGPL FFmpeg build
```

```
cargo test -p ove-core -p ove-store -p ove-projection -p ove-cli -p ove-playback   # document core (no FFmpeg needed)
source tools/ffmpeg/env.sh && cargo test --workspace                                # everything
cd apps/desktop && pnpm install && pnpm tauri dev                                   # run the app
```
