# Proposal: add-agent-chat-context-and-activity

## Why

Two gaps make agent work hard to follow and hard to direct.

**Telling the assistant what you mean.** There is no way to point at things in the chat:
- The person cannot point the assistant at a clip, a media file or a time range from the chat.
- They describe it in words, and the assistant must print the timeline to find it.
- Panels that know the context (captions, audio, media) cannot hand a prepared request to the assistant.
- Thinking is shown only as a "Thinking…" label.

**Seeing what an agent did.** Today:
- After a turn, "Show" outlines the items that the in-app assistant changed.
- Nothing is live while the assistant works.
- Edits by external agents (Claude Code, Codex and others over MCP or the CLI) are not shown at all.
- History lists revisions but does not group them by agent.

## What Changes

**Mentions and context**
- @mentions in the chat input pick media, timeline items or the selected range:
  - A mention is sent to the assistant as the exact printed lines of those items and the `rev` they came from.
  - With that, the assistant can edit them without printing the timeline first.
  - Mentions are saved in the chat log as structured records.
  - Image media that is mentioned is attached as an image.
- Panel hand-off: the Captions, Audio, Media and Effects panels can open the chat with a prepared request and
  mentions. Examples: "remove filler words from these captions", "score this range". The person edits the request and
  sends it. All prompt text is our own.
- A `/` menu in the chat input lists installed skills and project templates. Choosing one inserts a reference to it.

**Chat display**
- Streamed reasoning shows as a collapsible block, coalesced to the display frame rate.

**Live agent activity**
- While any agent works, the timeline shows:
  - a band on the ruler over the time range it is reading;
  - outlines on the items it reads or changes;
  - a badge with the agent's name.
- This covers the in-app assistant and external agents that edit through the running app.
- Bursts are coalesced. The display fades a few seconds after the last activity.

**History by agent**
- The History panel groups revisions by actor (person, assistant, each external client).
- It offers "Undo this agent's changes", which uses the actor-scoped undo from unify-agent-tool-layer.

**Out of scope:**
- Voice input.
- Chat sync between machines.
- Multi-agent orchestration.
- New agent tools beyond an activity notification.
- In-app chat through ACP agents (add-local-agents-and-onboarding section 3).

**References:**
- docs/spec/agents.md: UI tools, prompt rules.
- docs/spec/ipc.md: renderer–main events and the control socket.
- docs/spec/document-model.md: chat log.

## Capabilities

### New Capabilities
- `assistant-chat-context`: mentions of media, items and ranges sent as editable anchors; panel hand-off of prepared
  requests; the `/` menu of skills and templates; collapsible streamed reasoning.
- `agent-activity`: live display of what any agent reads and changes on the timeline, with actor badges; History grouped
  by actor with "undo this agent's changes".

### Modified Capabilities
- None.

## Impact

- UI:
  - `ui/src/ChatPanel.tsx`, plus a new mention popover and `/` menu.
  - `Timeline.tsx` gets an activity layer.
  - `HistoryPanel.tsx` groups revisions by actor.
  - Panels get a hand-off call.
- `src-tauri`:
  - The chat log in `chat.rs` gains mention records.
  - A new `agentActivity` event.
  - A control-link `notify` method for reads by external agents.
- **Depends on unify-agent-tool-layer:**
  - receipts and the actor on revisions;
  - stale-tolerant edits, which absorb mentions that were taken from an older `rev`;
  - actor-scoped undo.
- **Depends on add-agent-skills-and-connectors:** the skill list for the `/` menu.
