# Build plan — an agent-native editor modelled on the reference app, for a different use case

Status: DRAFT. The target use case is not decided yet. Every place marked [DOMAIN] changes once it is.
Read 01-reference-teardown.md first; section numbers below refer to it.

---------------------------------------------------------------------------------------------------------------------
## 0. What we copy, what we change

Copy (proven, it is the core of why the reference app works):
- Document-as-source: a literal, parse-only text projection of the document (`<doc>.tsx` or similar) with a rev
  header, item ids, item-level 3-way merge, revisions + undo, and `pending` placeholders (teardown §1).
- One editing pipeline for everyone: GUI, built-in agent, user's own agent, CLI and MCP all go through
  print → edit → check → apply.
- CLI as the agent's hands; the app's own MCP server only for UI moments (ask_user, pickers). Exit-code contract.
- Per-project AGENTS.md + typings file + shipped docs/techniques the agent reads on demand.
- Recipes: a template folder (instructions + structure + slots + locked templates) that narrows the agent's job.
- Verification loop for the agent: `verify` (lint) and `look` (render what the user sees) instead of "trust me".
- Tabs with sleeping renderers; background agent edits that never steal focus.

Change (from teardown §5):
1. Intake before agent: recipe/project inputs are a typed form, validated before the first agent turn. The agent
   only asks follow-ups, and an ask_user wait never keeps a turn (and its context) running.
2. Turn budgets: per-turn token/time caps shown to the user, resumable turns, summarised history instead of
   replaying 1.7 M tokens.
3. Lighter install: no 277 MB bundled agent binary; the hosted agent runs server-side, the desktop calls it. Ship
   matched, current toolchain binaries only where the domain needs them.
4. One credit unit, plain pricing.
5. Offline-capable core: local document, local render/preview; cloud only for heavy AI jobs.
6. Email + Google + Apple sign-in, no invite wall at GA.

---------------------------------------------------------------------------------------------------------------------
## 1. Architecture

Runtimes: latest LTS only (Node 26 LTS — Homebrew node is now 26.13.0, Electron current stable, React 19,
TypeScript current).

```
apps/
  desktop/        Electron shell: window, tabs (WebContentsView per tab, sleep/wake), typed RPC bridge,
                  URL scheme, file associations, auto-update (own channel: beta/stable)
  web/            same renderer served as a web app (if web is in scope)
  cli/            `<cli>` — print/check/apply, verify, look, history/undo, app control, docs, skills, mcp, auth
packages/
  doc-model/      [DOMAIN] schema (zod), ids, revisions, item-level 3-way merge, diagnostics  ← core, built first
  doc-source/     printer + parse-only parser for the text projection, `--eval` script runner
  render-core/    [DOMAIN] renderer for preview + export (deterministic: same input → same pixels/output)
  library/        bundle layout on disk, caches, media/asset ingest, relink, library move
  agent-runtime/  hosted-agent client, BYO-agent launcher (Claude Code / Codex), chat log store, skills
  app-mcp/        UI-moment tools: ask_user, pick_<asset>, confirm_spend
  ui/             design system (dark + light), panels, timeline/canvas widgets [DOMAIN]
services/ (cloud)
  api/            auth (OAuth + device flow for CLI), account/plan/credits, recipes catalog, uploads, share links
  agent-gateway/  LLM proxy for the hosted agent (metering, caps, model routing), turn records
  jobs/           queue + workers for heavy AI [DOMAIN: e.g. transcription, generation, cut-outs]
  export-pool/    optional cloud render
  mcp-remote/     remote MCP for ChatGPT/Claude connectors (same tool surface as `cli mcp`)
```

Contracts to fix before coding:
- RPC: one request/response/event channel, zod-validated, method registry generated from schemas (the reference app has 76
  handlers + 11 events; we start with the subset in §3).
- Bundle layout: `<Name>.<ext>/` with `project.json` (canonical), `<doc>.tsx` (projection), `<doc>.d.ts`, `AGENTS.md`,
  `media/`, `cache/`, `.revs/`, `agent/chats/*.jsonl`, `recipe/`, `.scratch/`, `.context/`.
- CLI exit codes: 0 ok, 1 refused (errors), 3 not signed in / app not running / not open, 4 out of credits.

---------------------------------------------------------------------------------------------------------------------
## 2. Algorithm work to design and validate BEFORE implementation

1. Merge engine (doc-model): 3-way item-level merge (base rev, file, current). Spec the conflict rules, id
   stability, deletes vs concurrent edits, ordering within tracks/containers. Validate with a property-based test
   suite (random concurrent edit pairs → invariant checks) before any UI.
2. Projection round-trip: print(parse(print(doc))) == print(doc); unknown attributes round-trip untouched.
3. Placeholder lifecycle: pending → filled (same id) → verify count = 0 → export allowed.
4. Render determinism [DOMAIN]: golden-frame/golden-output tests so `look` and export agree.
5. Agent turn budget + resume semantics: what is persisted, what is summarised, what a resumed turn sees.

---------------------------------------------------------------------------------------------------------------------
## 3. Screen inventory for v1 (mapped from the reference app, teardown §3) — must + useful only

| # | Screen | Keep from the reference app | Change |
|---|--------|---------------------|--------|
| 1 | Sign in | browser OAuth + code paste, CLI device flow | + email, Apple; no invite wall |
| 2 | Welcome | 2 questions max (role, what will you make) | drives default recipes |
| 3 | Home | prompt box, recipe strip, recent projects, what's new | intake form opens from prompt/recipe |
| 4 | Recipe detail | inputs, works-best-with, sample media, credit note | typed intake form, validated |
| 5 | Projects | search, rename, delete→Trash, multi-select | — |
| 6 | Editor: canvas | [DOMAIN] preview surface | — |
| 7 | Editor: structure view | [DOMAIN] timeline-equivalent (tracks/layers/sections), pending shimmer | — |
| 8 | Editor: left rail | Library + [DOMAIN] asset/tool panels | only panels the domain needs |
| 9 | Editor: properties | selection-driven inspector, presets, "ask agent to change this" | — |
| 10 | Editor: chat | agent/model picker, history, queued msgs, work log toggle, question cards, feedback | turn budget meter, resume |
| 11 | Export | [DOMAIN] formats, quality, plan gating | post-export checks kept |
| 12 | Settings | Library, Agents (BYO + tools install), Account, Usage, Shortcuts (presets), Privacy | single credit view |
| 13 | Global | tab bar, crash-safe tab reload, relink missing files, report issue, update banner | — |

Left out of v1 unless the domain needs them: developer overlays, DevTools dock, NLE exports, voice cloning,
remote MCP for ChatGPT/Claude (phase 4).

---------------------------------------------------------------------------------------------------------------------
## 4. Phases and acceptance criteria

P0 — Decide [DOMAIN] (blocking): users, output artefact, document elements, render target, heavy AI jobs.
     Exit: doc-model element table (like teardown §1 element set) and 3 example documents written by hand.

P1 — Core engine (no UI): doc-model, doc-source printer/parser, merge, revisions, verify.
     Exit: round-trip + merge property tests pass; CLI `print/check/apply/verify/history/undo` work on bundles.

P2 — Desktop shell + editor: Electron shell, typed RPC, library worker, tabs with sleep, editor screens 6–9 and 11,
     file-watch apply (GUI edits regenerate the projection; projection saves apply live).
     Exit: edit the same project from GUI and from a text editor at once with no lost changes, verified in the real
     app (real-browser/real-app test, not unit tests only).

P3 — Agent: hosted agent via agent-gateway, BYO Claude Code/Codex, app-mcp (ask_user etc.), AGENTS.md + skill +
     docs, intake forms, recipes (2–3 for [DOMAIN]), `look` for the agent, turn budgets.
     Exit: a recipe run end to end from intake to export with no manual fixes; token use per run measured and
     capped.

P4 — Accounts and money: auth, plans, single credit unit, usage page, upsells at limits, auto-update channels,
     telemetry opt-out. Optional: remote MCP connector.
     Exit: sign up → paid plan → credits debit on a real job → top-up, tested against the payment provider's test mode.

P5 — Hardening: crash-safe tabs, relink, offline mode, signed/notarised DMG, update rollback, report-issue flow.

---------------------------------------------------------------------------------------------------------------------
## 5. Open decisions (need your answer)

1. [DOMAIN] What the app is for: who uses it and what they produce.
2. Platforms for v1: Mac only, Mac + web (shared renderer, like the reference app), or Mac + Windows.
3. Agent power: hosted (we pay and meter), BYO (user's Claude/Codex), or both (the reference app does both).
4. Brand name (separate naming task).
