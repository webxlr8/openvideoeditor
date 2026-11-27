# Tasks: unify-agent-tool-layer

## 1. Tool registry

- [x] 1.1 Create crate `ove-tools` with `ToolSpec`, the annotations, surfaces and the `ToolCtx` trait, plus `BundleCtx`,
      `AppCtx` and `LinkCtx`. Verify with unit tests: each context runs a read-only tool on a fixture bundle.
- [x] 1.2 Move the 33 MCP tools and the 9 in-app tools into the registry, under one name per tool (keep old names as
      aliases for one release). Verify: a `tools/list` golden file per surface, and a parity test that the same input
      on `BundleCtx` and `AppCtx` gives the same receipt JSON.
- [x] 1.3 Turn `crates/ove-agent/src/tools.rs` and `crates/ove-cli/src/mcp.rs` into adapters, and route CLI
      subcommands to the same handlers. Verify: the existing `crates/ove-agent/tests/agent.rs` and CLI integration
      tests pass unchanged, and a snapshot test pins tool order and text.
- [x] 1.4 Document the registry and the tool list in `docs/cli/` and the agent guide (`ove docs agents`). Verify: the
      docs' examples run in the integration tests.

## 2. Edit contract and receipts

- [x] 2.1 Add the text-edit mode to MCP `apply_timeline`, and keep whole-file mode. Verify with integration tests for
      both modes, including the `edit.anchor` refusal.
- [x] 2.2 Make stale `rev` edits print the old revision, apply the edits, then 3-way merge, with an LRU of 16 printed
      revisions. Verify:
      - a property test (2,000 cases): edits against rev N−k, with concurrent GUI ops in between, give the same document
        as a manual rebase;
      - an `ove-bench` case on 5,000 items: apply against rev − 10 in ≤ 150 ms. Record the number in Results.
- [x] 2.3 Extend the apply plan and receipt with `changed` (printed lines), `removed`, `shifted`, `trackOrderChanged`
      and `rev`. Verify:
      - golden receipts;
      - a test that every returned line is a valid old-text anchor for the next edit;
      - a scripted 50-edit MCP session sends ≥ 5× fewer input bytes than whole-file mode. Record in Results.

## 3. Actor attribution and scoped undo

- [x] 3.1 Add the optional `actor` to the revision record, and fill it from the control link, `ove mcp` (`--turn`,
      client name) and the in-app assistant. Verify:
      - a migration test that opens bundles from the previous release;
      - a fuzz target for the record decoder that runs in CI.
- [x] 3.2 Move selective undo from `assistant.rs` into `ove-store`. Add scopes mine, turn and session (any only from the
      CLI with `--any`). Make `undo_timeline` default to mine for agents, and add `undo_my_changes`. Verify:
      - integration tests for the person-edits-survive and conflict-refused scenarios;
      - "Undo this turn" still works in the app's e2e test.
- [x] 3.3 Show the actor (kind and client name) per revision in the History panel. Verify: a UI test where a revision
      made over MCP shows the client name.

## 4. Session binding

- [x] 4.1 Add `ove mcp --project <p> [--turn <id>]` and the `use_project` tool, and refuse other projects with
      `project.pinned`. Verify: a scripted client runs two concurrent pinned sessions on different projects, and each
      project's history shows only its own session.
- [x] 4.2 Allow writes to an open project that is not in front. Verify: an e2e test where the person's window shows
      project Q and the MCP session edits project P; P's history gains the revision, and Q is untouched.
- [ ] 4.3 Keep the bundle open for the whole `ove mcp` session, closing it when idle and keeping the existing lock
      (gate 1, D3: the headless stale-apply case measured 207 ms against a 150 ms budget; the extra 95 ms is opening
      the bundle and writing the snapshot on close, not printing). Verify: the `stale-apply` bench case, headless with
      the revision cached, at 150 ms or less; the lock-contention test in `mcp_edits.rs` still passes while the app
      opens the same project.

## 5. Frames for agents

- [x] 5.1 Add the grid overlay (transform units, safe boxes, built-in bitmap digits) to `look` and `grab_frame` in
      `ove-analyze`. Verify:
      - golden PNGs;
      - the grid-units scenario on a fixture;
      - a golden test that export frames are unchanged.
- [x] 5.2 Add storyboard mode (keyframe-snapped sampling, near-duplicates dropped, at most 36 tiles, timecodes, visible
      item ids on timeline sheets). Verify: on a 10-minute fixture, at most 36 tiles and at least one tile per shot
      reported by `detect_scenes`.
- [x] 5.3 Expose the frame and analysis tools to the in-app assistant. Verify: an agent test with a mock model whose turn
      calls `look` and receives an image block.

## 6. Generation tools

- [ ] 6.1 Add `list_models`, `generate_*`, `generation_status` and `cancel_generation` to the registry, using the
      existing `ove-generate` jobs. Verify: tests against local stand-in providers, where the placeholder fills and can
      be undone.
- [ ] 6.2 Implement quotes: request-hash binding, 10-minute expiry, and the `confirm_spend` control method with a price
      card in the app, MCP elicitation, and the in-app question card. Verify:
      - the hostile-agent test (`confirm.required`);
      - an expired-quote test;
      - an e2e test where approving the card starts the job.
- [ ] 6.3 Add a per-session spend cap in Settings (default $5) and the `spend.cap` refusal. Verify: a unit test, and a UI
      test where changing the cap takes effect.

## 7. Local feedback queue

- [ ] 7.1 Implement `send_feedback` (local JSON queue, duplicate drop, limit of 8) and list the queue in Help → Feedback
      with edit, delete and send. Verify:
      - unit tests for duplicates and the limit;
      - an e2e test that the report appears and that no network call is made (network blocked in the test).

## 8. Integration checks

- [ ] 8.1 On built apps on macOS and Windows:
      - Claude Code connects through `ove mcp --project`, makes 20 edits without printing again, and undoes its own
        changes, while the person's edits remain.
      - Record the results.
- [ ] 8.2 On both OSes, a full generation flow: agent quote, then price card, then approval, then placeholder fill, then
      undo. Record timings in Results.
- [ ] 8.3 Prompt-cache check: the in-app cache hit rate across 10 turns is within 5% of the rate before this change.
      Record the numbers in Results.
