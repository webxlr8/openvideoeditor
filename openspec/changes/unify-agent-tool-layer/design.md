# Design: unify-agent-tool-layer

## Context

See proposal.md (Why). This change builds on:
- `timeline-projection`: print, the text-edit apply, and the 3-way merge.
- `project-document`: revisions and inverse ops.
- The control link (`crates/ove-control`).
- The in-app assistant (`crates/ove-agent`).

Contracts: docs/spec/mcp-tools.md and docs/spec/agents.md. Requirements: specs/agent-tool-layer.

Current state:
- `ove-agent::tools` defines 9 tools behind a `ToolHost` trait.
- `ove-cli::mcp` defines 33 tools as a table that maps each tool to CLI argv.
- `RevisionRecord` has `source` and no actor.
- `assistant.rs::undo_turn` applies the inverse ops of one turn's revisions as one new revision, and refuses when there
  is a conflict.

## Goals / Non-Goals

**Goals:**
- One definition per tool, so every surface behaves the same and cannot drift.
- An agent can make 50 consecutive edits without printing the timeline again.
- No agent can undo another actor's work by accident, and no agent can spend money without a person's confirmation.

**Non-Goals:**
- Replacing the TSX edit contract with many small clip tools. Add intent tools only where TSX is a poor fit; these come
  in later changes.
- A network transport other than stdio and the existing loopback control link.

## Decisions

### D1. A registry crate, three contexts
`ove-tools` holds `ToolSpec` values, each with:
- `name`
- `description`
- `schema`: hand-written JSON Schema
- `annotations`: readOnly, destructive, spends, needsApp
- `surfaces`: inApp, mcp, pinned
- `handler(ctx, args) -> Receipt`

A handler sees only a `ToolCtx` trait. Three types implement it:
- `AppCtx`: the open project in the desktop app.
- `BundleCtx`: a project on disk, opened headless.
- `LinkCtx`: forwards to the running app over the control link.

The adapters are small:
- In-app: the tool list, plus `ask_user` handled by the UI.
- MCP: JSON-RPC `tools/list` and `tools/call`.
- Pinned: MCP with a fixed project and turn.

CLI subcommands call the same handlers, so the rule "CLI and tools agree" holds by construction.

Alternatives considered:
- Keep three sets and add a parity test. Rejected: drift is caught, not prevented.
- Make MCP the only surface and run the in-app assistant through it. Rejected: an extra process hop per tool call, and
  the in-app UI tools (`ask_user`, `seek_viewer`) need app state.
- Generate schemas from Rust types. Rejected for now: hand-written schemas give better descriptions for agents and avoid
  a derive dependency. Revisit when the set reaches about 80 tools.

### D2. Stale-tolerant text edits
The edit contract is `{project, rev, edits:[{oldText, newText}]}`. Applying a request takes these steps:
1. Print the projection of `doc_at(rev)`. This is the file that the agent saw.
2. Apply the text edits to that print.
3. Run the existing 3-way merge, with base `doc_at(rev)` and the current document.

A request is refused in two cases:
- An `oldText` does not match the old print. Code: `edit.anchor`.
- The merge reports a conflict and the caller asked for `prefer: "editor"`.

The last 16 printed revisions are cached per project, so most applies do not print again.

Alternatives considered:
- Refuse any stale `rev`, as today. Rejected: agents waste a round trip on every concurrent GUI edit.
- Apply text edits to the current print. Rejected: `oldText` may no longer be unique or present, so edits could land on
  the wrong item.

### D3. Receipts
`Plan` and the apply result gain these fields:
- `changed`: the id and freshly printed line of each item that was added or updated, capped at 30, with a count of the
  rest.
- `removed`: id and name of each removed item.
- `shifted`: groups of three or more items on one track that moved by the same delta, reported as
  `{track, from, by, count}` instead of one row per item.
- `trackOrderChanged`.
- `rev`.

The printed line is the exact text the next `oldText` anchor needs.

Field names in the receipt JSON: `changed` and `removed` already meant "a revision was made" (true/false) and "how
many items were removed" in the command line's `--json` output, so the lists are `changedItems` (`{id, text}`, where
`text` is the item's printed element, one line or the lines of an element written one attribute per line),
`moreChanged` (the count beyond 30), `removedItems` (`{id, name}`, as before), `shifted` and `trackOrderChanged`. An
item that only moved as part of a shift rule is not listed in `changedItems`.

Alternatives considered:
- Return the whole new print. Rejected: large on long projects.
- Return field-level JSON diffs. Rejected: the agent would have to rebuild the TSX text from them before its next edit.

### D4. Actor on revisions
`RevisionRecord` gains `actor: Option<{kind: user|assistant|cli|mcp|template|generation, session?, turn?, client?}>`:
- Records written before this change read `actor` as empty (`serde(default)`).
- `control.rs` fills `actor` from the authenticated socket session.
- `ove mcp` fills `actor` from `--turn` and from the `initialize` client name.

Selective undo moves from `assistant.rs` into `ove-store`. It collects the caller's revisions in the requested scope,
composes their inverse ops in reverse order, and applies them to the current document as one revision. It refuses on
conflict and lists the conflicting items. The scopes are:
- `mine`: the default for agents.
- `turn`.
- `session`.
- `any`: needs the CLI flag `--any`. It is not available to agents.

Alternatives considered:
- A separate undo stack per actor. Rejected: two sources of truth.
- Take the actor from `source` only. Rejected: it cannot tell two MCP sessions apart.

### D5. Session binding
`ove mcp --project <p> [--turn <id>]` binds the session to one project:
- Every project-scoped tool uses the bound project.
- A call that names another project is refused with `project.pinned`.

Without the flag, `use_project` sets a session default.

Unlike a "frontmost project" rule, writes to a project that is not in front are allowed. They merge as normal
revisions.

Alternatives considered:
- Bind to the frontmost window. Rejected: it breaks headless use and causes refusals while the person works in another
  project.

### D6. Grid and storyboard frames
The grid overlay is drawn only in the analysis path, never in export:
- Ticks every 10% of the canvas.
- Labels in the timeline transform units: pixels from the canvas centre, the unit `transform={{x,y}}` uses.
- Title-safe and action-safe boxes.
- Labels come from a small built-in bitmap font of our own design.

Storyboard mode:
- Sample about 120 frames, snapped to decoder keyframes.
- Drop a frame when its 8×8 luma grid differs from the last kept frame by less than a threshold.
- Keep at most 36 tiles in 6 columns.
- Burn in the timecode, and on timeline sheets the ids of the visible items, read from the document at that time.

Alternatives considered:
- A normalised 0–1 grid. Rejected: the agent would have to convert to our units by hand.
- Evenly spaced tiles. Rejected: they miss short shots and repeat static ones.

### D7. Generation quote and confirmation
A generation call goes through these steps:
1. A call without `confirm` returns `{plan, estimateUsd, quoteId}`. A quote expires after 10 minutes and is tied to a
   hash of the request.
2. A call with `confirm: quoteId` spends only after the app has recorded that a person approved that quote.
3. The person approves through one of three routes:
   - The running app shows a price card through a new control method, `confirm_spend`.
   - The in-app assistant uses the `ask_user` card.
   - Headless `ove mcp` uses MCP elicitation when the client supports it. Otherwise the call is refused with
     `confirm.required`.

The agent's own `confirm` argument is never enough on its own. A per-session cap (Settings, default $5) refuses calls
above it.

Results arrive as `pending` placeholders, through the existing generation worker.

Alternatives considered:
- Trust a `--yes` from the agent. Rejected: an agent could spend without the person knowing.
- Block every call on a modal dialog. Rejected: headless use would be impossible.

### D8. Local feedback queue
`send_feedback {category, severity, summary}`:
- Writes `<config>/feedback/<ts>.json` with app and OS versions, the last 15 tool names and the last error code.
- Drops exact duplicates within a session and accepts at most 8 per session.
- Never sends anything.

Help → Feedback lists queued entries. The person can edit, delete or send each one, as a prefilled issue URL or a copied
report.

Alternative considered: send directly. Rejected: v1 has no accounts and no network telemetry.

## Risks / Trade-offs

- **The prompt-cache prefix changes once** when the tool definitions move. Mitigation: a snapshot test pins tool order
  and text. Detection: the test fails on any change, and `Usage` cache-read tokens drop.
- **Printing old revisions is slow on long projects.** Mitigation: an LRU of printed revisions. Detection: an `ove-bench`
  case that applies against rev − 10 on a 5,000-item project, with a budget of 150 ms.
- **Log format change.** Mitigation: an additive optional field, a migration test on bundles written by the previous
  release, and a fuzz target for the record decoder in `fuzz/`. Detection: CI fuzz and migration tests.
- **An agent tries to confirm its own quote.** Mitigation: the approval is stored only by the UI or elicitation path.
  Detection: a test with a hostile stand-in agent.
- **Grid labels leak into export.** Mitigation: the overlay exists only in `ove-analyze`. Detection: a golden test that
  export frames are unchanged when grid mode is used during the same session.

## Migration Plan

Additive. Old bundles open unchanged. The MCP whole-file `source` mode stays. The `undo_timeline` default changes from
"latest" to "mine" for agents. docs/cli and the agent guide document this.

## Open Questions

- **Wave order:** waves proceed in the master-plan order. This change is wave 1 (default until the owner decides
  otherwise).
- **Model downloads:** models and runtimes download on first use, with explicit consent above 50 MB. This change adds no
  models.
- **DirectML:** off by default, with CPU fallback, pending a licence ruling. Not used here.
- **SVT-AV1:** excluded pending a licence ruling. Not used here.
- **Remote model catalog:** an embedded copy plus an optional signed remote URL setting; host to be decided.
  `list_models` reads the embedded catalog until extend-ai-generation-actions lands.
- **Community skill catalog:** the URL is configurable and 3 original starter skills ship
  (add-agent-skills-and-connectors).
- **Session spend cap:** the default of $5 is a placeholder for the owner to confirm.

## Results

### Group 1: tool registry (Windows 11 x64, debug build)

- The registry holds 35 tools: the 33 former MCP tools, `check_code` (now also on MCP) and `ask_user` (in-app
  only). Surfaces: in-app 9 tools, MCP 34, pinned 32 (`list_projects` and `create_project` are not on the pinned
  surface, which works on one project).
- The in-app assistant's former names `undo_last`, `import_media` and `seek_viewer` are aliases of `undo_timeline`,
  `add_media` and `seek`. The in-app tool list changed once, as the design's prompt-cache risk expects; the golden
  file `crates/ove-tools/tests/golden/tools-inApp.json` pins it from now on.
- The MCP adapter no longer turns calls into command-line arguments and temporary files: it validates against the
  registry and runs the handler, and image tools return PNG bytes directly.
- Unchanged tests: `crates/ove-agent/tests/agent.rs` 14 passed (1 ignored, live API), `crates/ove-cli/tests/*` all
  passed, including `media.rs` with fixtures made by a full FFmpeg CLI (`OVE_FIXTURE_FFMPEG`).
- Implementation notes:
  - The in-app list grew to 10 tools with `undo_my_changes` (group 3); the golden file pins it.
  - `BundleCtx` takes a project's write lock; `LinkCtx` adds the control link (edits to a project the app holds go
    through the app, with the 3-attempt retry on `rev.changed`); `AppCtx` runs on the app's open project through an
    `AppHost` the desktop implements.
  - Command-line-only arguments (output paths, `--format`, `--image-duration`) are part of the tool's table but are
    marked command-line only: agents never see them, and an agent that passes one gets `argument.unknown`.
  - An unknown tool over MCP stays a JSON-RPC error (-32602), as before; its `data` carries code `tool.unknown`
    and the closest names.
  - `verify_timeline` refuses with `verify.failed` when there are errors on every surface (the in-app assistant
    used to get them as a normal result); the findings are in the refusal.

### Group 2: edit contract and receipts (Windows 11 x64)

- Rebase property test (`crates/ove-tools/tests/edits.rs`, `stale_edits_match_a_manual_rebase`): 2,000 cases, each
  with 0–5 person edits (moves, renames, text changes, including the same field the agent changes) between the
  agent's print and its edit; every case gives the same items and tracks as setting the agent's values on the
  current document. Run time 354 s in a debug build (each case creates a project bundle on disk).
- `ove-bench` case `stale-apply` (release build, 5,000 items on 5 tracks, one text edit against rev − 10, 7 runs,
  median):

  | Where the project is | Printed rev cached | Not cached |
  |---|---|---|
  | Held open, as the app holds it (`AppCtx`) | 112 ms | 181 ms |
  | Opened and closed per call (`BundleCtx`, headless `ove mcp`) | 207 ms | 260 ms |

  The 150 ms budget is met for the app and for the usual agent case (the agent printed the revision in this
  session). Headless calls also pay for opening the bundle and for the snapshot written when it closes; keeping
  bundles open across calls in `ove mcp` is the next step if that path needs the same budget. Before the document
  cache, the held and cached case measured 468 ms: the base document was rebuilt from the revision log (48 ms) and
  the whole timeline was printed again for the receipt (31 ms) on every call. Now the last 16 revisions' printed
  text and documents are cached per project, and the receipt prints only the changed items.
- Scripted 50-edit MCP session on a 60-item project (`crates/ove-cli/tests/mcp_edits.rs`): text edits anchored on
  the receipts' lines sent 20,457 bytes; resending the whole file sent 398,267 bytes: 19.5× fewer.

### Group 3: actor attribution and scoped undo

- `RevisionRecord.actor` is optional (`serde(default)`, not written when empty), so records from earlier releases
  read as before. An actor kind written by a newer build reads as `other` instead of making the record unreadable.
- Selective undo lives in `ove-store` (`Project::undo_selected`): the inverse operations of the chosen revisions,
  newest first, applied as one revision. A later revision outside the set that touched the same object field (or
  added or removed the object) refuses it with the conflicting items. "Undo this turn" in the app uses it.
- Agents: `undo_timeline` undoes the session's latest change that is not undone yet; a revision of another actor is
  refused with `undo.notYours`. `undo_my_changes` takes scope `session` or `turn`; scope `any` is refused for agents
  (`undo.scope`) and reached from the command line as `ove undo <project> --any --session <id>`. The command line's
  plain `ove undo` keeps its meaning (latest revision, whoever made it, or back to `rev`): it is the person's own
  command.
- Sessions: each `ove mcp` process is one session (`mcp-<time>-<pid>`), with the client name from `initialize`;
  the in-app assistant's session is the chat and its turn the turn id; the control link accepts actors of kind
  `cli` and `mcp` only.
- The fuzz target `revision_record` is in the nightly fuzz matrix; it was compiled here but not run (no nightly
  toolchain with cargo-fuzz on this machine).

### Group 4: session binding

- `ove mcp --project <p> [--turn <id>]` serves the pinned surface (34 tools, MCP has 36; no
  library listing or project creation). `use_project` sets the default project of an unbound session.
- Two pinned `ove mcp` processes on projects P and Q, editing in alternation (3 edits each), each refused when
  naming the other project: each project's history shows the 3 revisions of its own session only, each with its
  `--turn` id (`crates/ove-cli/tests/mcp_pinned.rs`).
- Task 4.2 is open: the app holds one project at a time (`projects.open`), so "a project open in the app but not in
  front" needs multi-window support first; through the control link, a write to a project the app does not hold
  goes to the bundle on disk, and one to the held project goes through the app.
