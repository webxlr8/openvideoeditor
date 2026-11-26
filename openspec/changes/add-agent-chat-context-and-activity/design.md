# Design: add-agent-chat-context-and-activity

## Context

See proposal.md (Why). This change builds on these parts of unify-agent-tool-layer:
- the tool registry;
- receipts that carry printed lines;
- the actor on revisions;
- actor-scoped undo.

It also builds on add-agent-skills-and-connectors (the skill index).

Contracts: docs/spec/agents.md and docs/spec/ipc.md. Requirements: specs/assistant-chat-context and
specs/agent-activity.

Current state:
- `ChatPanel.tsx` has starter prompts, a per-turn "Show" (the `reviewed` class in `Timeline.tsx`) and "Undo this turn".
- `thinkingDelta` only toggles a label.

## Goals / Non-Goals

**Goals:**
- A mention lets the assistant act on an item in the same turn, with no print round trip.
- A person can always see which agent is touching which part of the timeline.

**Non-Goals:**
- Showing an agent's private reasoning beyond what the provider streams.
- Any network transmission of activity data.

## Decisions

### D1. A mention is a TSX excerpt
When a message is sent, each mention is resolved to one of three forms:
- **Item:** the item's printed line(s) from the current projection, plus `rev`.
- **Range:** start and end times, plus the printed lines of the items that overlap the range, capped at 40 lines with a
  count of the rest.
- **Media:** the media table row. Images are also attached, downscaled to a long side of 1,568 px.

The excerpt is frozen at send time and stored in the chat log as `{kind, ids, rev, excerpt}`. If the person edits the
item after sending, the assistant's edit against the older `rev` is rebased by the stale-tolerant apply from
unify-agent-tool-layer.

Alternatives considered:
- Send only ids, so the assistant must print. Rejected: one extra round trip on every mention.
- Send a JSON blob of fields. Rejected: the assistant would have to rebuild the TSX text to edit it.

### D2. Panel hand-off
There is one UI call: `askAssistant({prompt, mentions, newChat})`. Each panel defines its own short prompts in a
localisable string table:
- Captions: filler words, names and jargon, translation.
- Audio: score this range, duck under dialogue.
- Media: organise media.
- Effects: match this shot to the selected reference.

The call opens the chat and fills the draft. It never sends automatically.

Alternative considered: send automatically. Rejected, because the person should see and change the request before it
costs money.

### D3. The `/` menu
Typing `/` at the start of the input lists installed valid skills and project templates, filtered as the person types.
Choosing a skill inserts a skill chip; the turn then tells the assistant to read that skill first. Choosing a template
inserts the template's prompt and opens its intake form if it has one.

Alternative considered: put skills only in the system prompt index. Rejected: the person wants to choose a specific
playbook.

### D4. Collapsible reasoning
Reasoning deltas are buffered and flushed once per animation frame. They render in a collapsed block that shows the
first line and the elapsed time. The block is collapsed by default, and the setting is remembered per user.

Alternative considered: render every delta. Rejected: it causes layout churn at high token rates.

### D5. Activity events
`agentActivity {actor, kind: read|change, ids?, range?, ts}` events come from two sources.

Sources in the app:
- **The in-app tool layer.** A read tool maps its arguments to ids or a range:
  - a `print_timeline` line range maps to item ids, because the printer records which lines belong to which item;
  - `look --at` maps to a time range.
  - A change comes from the receipt's `changed` and `removed` ids.
- **The control link.** An `apply` request from an external agent produces change events. External reads report
  through a new fire-and-forget `notify` method that `ove` CLI and MCP read commands call.

How events are shown:
- Events are coalesced over 100 ms.
- The timeline draws a ruler band, item outlines and an actor badge.
- The display fades 3 s after the last event.
- If the person edits during an agent's change, the outline for that change is skipped, so that the person's own edit
  is not shown as an agent change.

Alternatives considered:
- Show activity only after a turn. Rejected: it does not show external agents or work in progress.
- Diff the whole document on each revision. Rejected: it cannot show reads.

### D6. History grouped by actor
The History panel groups consecutive revisions by actor. A group shows the actor's name: "You", "Assistant", or the
client name such as "Claude Code". Each agent group offers "Undo this agent's changes". This calls actor-scoped undo
with scope session, or turn for in-app turns. A conflict shows the conflicting items and changes nothing.

## Risks / Trade-offs

- **A mention goes stale before the agent uses it.** Mitigation: store `rev`; stale-tolerant apply rebases the edit.
  Detection: an agent test that edits a mentioned item after a GUI change.
- **Event floods during agent loops.** Mitigation: 100 ms coalescing and a cap of 200 outlined items, then a count.
  Detection: a test that runs 1,000 applies in 10 s and keeps the UI frame time under 16 ms.
- **Images in mentions use many tokens.** Mitigation: downscale, and limit to 5 images per message with a notice.
  Detection: a unit test for the limit.
- **Read notifications from old CLI versions.** Mitigation: `notify` is optional; a missing call only means no read
  band. Detection: none needed.

## Migration Plan

Additive. Chat logs without mention records open unchanged.

## Open Questions

- Waves proceed in master-plan order. This change is wave 3 (default).
- Models and runtimes download on first use, with explicit consent above 50 MB. This change adds none.
- DirectML off by default (CPU fallback) pending licence ruling. Not used here.
- SVT-AV1 excluded pending ruling. Not used here.
- Remote model catalog: an embedded copy plus an optional signed remote URL setting; host to be decided. Not used here.
- Community skill catalog: the URL is configurable, and 3 original starter skills ship. The `/` menu lists whatever is
  installed.

## Results

(To be filled during apply.)
