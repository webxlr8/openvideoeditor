# Proposal: unify-agent-tool-layer

## Why

There are three agent tool sets today, and they drift apart:

- The in-app assistant has 9 tools.
- `ove mcp` has 33 tools, each mapped to CLI arguments.
- The planned pinned server for local agents (add-local-agents-and-onboarding, D5/D6) has none yet.

Names differ between the sets (`seek_viewer` and `seek`). Some tools exist on one surface only (`check_code` in-app;
`look`, `grab_frame` and `filmstrip` over MCP only). The in-app assistant cannot see frames.

The two edit contracts also differ:

- In-app `apply_timeline` takes text edits and a `rev`.
- MCP `apply_timeline` takes only a whole replacement file, so an external agent resends all of `timeline.tsx` on every
  edit.

Edits against an old `rev` are refused, even when the 3-way merge could rebase them. Receipts do not return the ids of
new items or their printed text, so the agent must print the timeline again after each edit.

There are also gaps in safety and attribution:

- Revisions record a source but no actor (session, turn or client).
- Over MCP, `undo_timeline` undoes the latest revision, whoever made it.
- Generation is not available as an agent tool, although its CLI exists.

## What Changes

- **One tool registry.**
  - One registry describes every agent tool once: name, description, input schema, annotations (read-only,
    destructive, spends money, needs the app) and the surfaces it appears on.
  - The in-app assistant, `ove mcp` and the pinned server are thin adapters over it.
  - CLI commands call the same handlers.
  - Tool order is fixed and covered by a snapshot test, so the in-app prompt-cache prefix stays stable.
- **Edit contract.**
  - MCP `apply_timeline` accepts text edits plus `rev`, the same as in-app. Whole-file `source` stays as a second mode.
  - Edits made against an older revision are rebased through the existing 3-way merge. Only a real conflict is
    refused.
- **Receipts.** Every edit returns:
  - for each added or changed item, its id and printed line;
  - the removed ids and names;
  - compressed shift rules ("12 items on track V1 moved by +2.0 s");
  - whether track order changed;
  - the new `rev`.
- **Actor attribution.**
  - Each revision records an optional actor: kind, session, turn and client name.
  - `undo_timeline` defaults to undoing only the caller's own revisions, using the selective-inverse method that "Undo
    this turn" already uses.
  - A new `undo_my_changes` tool undoes all of a session's (or turn's) revisions.
- **Session binding.**
  - `ove mcp --project <p> [--turn <id>]` pins a session to one project.
  - A `use_project` tool sets the session default, so `project` becomes optional on later calls.
  - The client name from MCP `initialize` is recorded as the actor's client.
- **Visual self-check.**
  - `look` and `grab_frame` can overlay a coordinate grid in timeline transform units, with title-safe and action-safe
    boxes.
  - `filmstrip` and `look` gain a storyboard mode: dense sampling, near-duplicate frames dropped, timecodes burned in,
    and the ids of visible items burned into timeline tiles.
- **Analysis tools in-app.** The in-app assistant gets `look`, `grab_frame`, `filmstrip`, `detect_scenes`,
  `audio_levels` and `detect_silence` through the registry. This closes add-local-agents-and-onboarding task 3.3a.
- **Generation as tools.**
  - New tools: `list_models`, `generate_image`, `generate_video`, `generate_voice`, `generate_music`, `generate_sfx`,
    `generation_status` and `cancel_generation`.
  - Spending uses a two-step quote:
    - The first call returns a plan, an estimate and a quote id. It spends nothing.
    - Spending needs a confirmation by a person, made in the app, through MCP elicitation, or through the in-app
      question card.
  - A spend cap applies per session.
- **Feedback.** A `send_feedback` tool writes a paraphrased report to a local queue (de-duplicated, at most 8 per
  session). Nothing leaves the computer until a person sends it from Help → Feedback.

**Out of scope:**
- HTTP MCP transport.
- Chat UI changes, mentions and live activity (add-agent-chat-context-and-activity).
- Skills, the skill catalog and client connectors (add-agent-skills-and-connectors).
- Timeline edit planners such as ripple (add-edit-planners-and-links).
- The remote model catalog and AI clip actions (extend-ai-generation-actions).

**References:**
- docs/spec/mcp-tools.md: every tool listed, including generation, `undo_timeline` and `apply_timeline`.
- docs/spec/agents.md: prompt rules, UI tools, launch contracts.
- docs/spec/cli.md: `generate`, `undo`, `history`.
- docs/spec/document-model.md: Apply / merge, revisions.

## Capabilities

### New Capabilities
- `agent-tool-layer`: one tool registry on all agent surfaces; the text-edit contract with stale-tolerant rebase;
  structured edit receipts; actor attribution and actor-scoped undo; session project binding; grid and storyboard
  frames; generation tools with confirmation by a person; local feedback queue.

### Modified Capabilities
- None. This adds to `timeline-projection` and `project-document` without changing their requirements.

## Impact

- New crate `ove-tools`: the registry, the handlers, and the tool context for the app, a bundle on disk, and the control
  link.
- Rewrites:
  - `crates/ove-agent/src/tools.rs` and `crates/ove-cli/src/mcp.rs` become adapters over the registry.
  - The `crates/ove-projection` apply plan returns receipt data.
  - The `crates/ove-core` revision record gains an optional actor field.
- `crates/ove-control` forwards the actor and the confirmation requests.
- `crates/ove-analyze` gains the grid overlay and storyboard sampling.
- **Must land before:** add-local-agents-and-onboarding tasks 3.2 and 3.4, add-agent-skills-and-connectors,
  add-agent-chat-context-and-activity, and extend-ai-generation-actions.
- **Absorbs:** add-local-agents-and-onboarding tasks 3.3 and 3.3a, and the open MCP part of add-ai-generation task 1.2.
- **Changes the durable revision log format** (additive and backward-compatible).
