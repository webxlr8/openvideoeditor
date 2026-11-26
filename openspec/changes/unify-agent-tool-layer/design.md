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

(To be filled during apply with measured numbers.)
