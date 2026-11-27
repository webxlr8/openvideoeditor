# Tasks: add-agent-chat-context-and-activity

## 1. Mentions

- [x] 1.1 Build the @ popover (media, items, selected range; tabs All, Video, Image, Audio). Verify with Vitest tests of
      the popover filtering and keyboard selection.
- [x] 1.2 Resolve mentions at send time to excerpts: item lines with `rev`, range lines (capped at 40), media rows, and
      images downscaled and limited to 5. Store them in the chat log as structured records. Verify:
      - unit tests of excerpt building;
      - a chat-log round-trip test;
      - the too-many-images notice.
- [x] 1.3 Verify with an agent test using a mock model: a turn with an item mention edits it with no print call, and an
      edit after a concurrent GUI move merges. Document mentions in the in-app help.

## 2. Panel hand-off and `/` menu

- [ ] 2.1 Add `askAssistant` and the prepared requests in Captions, Audio, Media and Effects, with our own strings.
      Verify with a UI test per panel: the draft and mention appear, and nothing is sent.
- [ ] 2.2 Add the `/` menu over skills and templates, with a skill chip and template intake. Verify:
      - a UI test of filtering and the no-match path;
      - an agent test that the turn reads the chosen skill first.

## 3. Reasoning display

- [ ] 3.1 Buffer reasoning deltas per animation frame and render them in a collapsible block with elapsed time. Verify:
      - a UI test of expand and collapse;
      - a frame-time measurement while streaming at 200 tokens/s, under 16 ms per frame. Record in Results.

## 4. Live activity

- [ ] 4.1 Emit `agentActivity` from the in-app tool layer (line ranges mapped to ids, `look` mapped to ranges, receipts
      mapped to changes). Verify with unit tests of the mapping.
- [ ] 4.2 Emit change events for control-link applies, and add the optional `notify` method for external reads, called
      by `ove` read commands. Verify with an integration test: an MCP read and apply produce read and change events.
- [ ] 4.3 Draw the activity layer in the timeline: ruler band, outlines, actor badge, 100 ms coalescing, 3 s fade,
      skipped when the person is editing, capped at 200. Verify:
      - UI tests of each scenario;
      - the flood test (1,000 applies in 10 s) with frame time under 16 ms. Record in Results.

## 5. History by actor

- [ ] 5.1 Group revisions by actor in the History panel, and add "Undo this agent's changes" using actor-scoped undo.
      Verify with UI tests for the undo-one-agent and conflict scenarios.

## 6. Integration checks

- [ ] 6.1 On built apps on macOS and Windows:
      - mention an item and have the assistant edit it;
      - run Claude Code over MCP while the person watches the outlines and badge;
      - undo Claude Code's changes from History.
      Record the results.
