# Tasks: extend-ai-generation-actions

## 1. Catalog

- [ ] 1.1 Define the catalog schema and migrate today's 10 models into the embedded catalog; verify a test that every
      current CLI and MCP generation call produces the same request body as before.
- [ ] 1.2 Implement signature verification, schema-version gating, domain allow-list checks and the load order
      (embedded, cached, remote, once a day); verify unit tests for good, tampered, wrong-key, too-new and off-list
      catalogs, and an offline-start test.
- [ ] 1.3 Implement request templates and output pointers; verify golden tests that turn each catalog entry into its
      request body against a stand-in server.
- [ ] 1.4 Add the catalog signing step to release CI and the Settings controls (refresh off, local file, extra trusted
      key); verify a CI dry run signs and the app accepts the signed file.
- [ ] 1.5 Document the catalog format in `docs/spec/generation-catalog.md`; verify the schema in the doc is generated
      from the code with a drift check.

## 2. References, preprocessing and spend

- [ ] 2.1 Verify every (inferred) provider endpoint and upload flow against current provider documentation and record
      the result in design.md; verify by a stand-in contract test per endpoint built from those docs.
- [ ] 2.2 Implement `refs`, tag validation, uploads with the content-hash cache and lineage in `data.generated`; verify
      the two-references, missing-reference and upload-reused scenarios against a stand-in server.
- [ ] 2.3 Implement preprocessing through range export (trimmed range, timeline span, downscale, transcode, cleanup);
      verify the trimmed-range and timeline-span scenarios by probing the uploaded files.
- [ ] 2.4 Implement price rules, the spend ledger and budgets with the confirmation rule; verify unit tests per price
      rule and the agent-over-budget and ledger scenarios.

## 3. AI actions

- [ ] 3.1 Implement the action table and take / replace / audio-lane placement; verify the take, replace-keeps-edits
      and provider-failure scenarios against stand-in servers, each as one revision with one undo step.
- [ ] 3.2 Implement upscale, reframe / extend, lip sync, prompt edit, rerun and draft to enhance; verify a stand-in test
      per action.
- [ ] 3.3 Implement video to sound effects, video to music, dubbing (polled), voice cleanup, voice changer and voice
      clone with consent; verify a stand-in test per action and the clone-without-consent scenario.
- [ ] 3.4 Implement AI transitions (gap and cut, fit by speed or trim, take fallback); verify the gap-by-trim,
      gap-too-long and gap-filled-meanwhile scenarios; the fitted duration equals the gap to the frame.

## 4. Surfaces

- [ ] 4.1 Add `ove generate action|transition|models --refresh`; verify CLI integration tests including exit codes and
      the unknown-item error.
- [ ] 4.2 Add MCP `ai_clip_action`, `generate_transition` and catalog-driven `list_models` in the tool registry with
      quote and confirm; verify MCP contract tests including a refused unconfirmed call.
- [ ] 4.3 Rebuild the Generate panel from capabilities with the mention picker; add the timeline AI submenu, the
      Inspector AI section and take-lane show / hide; verify UI tests for unsupported-mode hiding and the unavailable
      reason.
- [ ] 4.4 Update `docs/cli/` generation pages and the agent guide with actions, transitions, budgets and takes; verify
      their examples run in the CLI integration tests against stand-ins.

## 5. Integration checks

- [ ] 5.1 In the built app on macOS and Windows with real keys: upscale a clip as a take, replace with a reframe,
      generate a transition in a gap, add video-to-sound-effects; verify placements, undo and the spend ledger.
- [ ] 5.2 Agent path on both OSes: an MCP client runs an action within budget, then one over budget that waits for the
      person's confirmation; verify both outcomes.
- [ ] 5.3 Weekly live smoke test with a maintainer key runs once against every catalog action; record pass / fail and
      estimate-versus-reported cost in design.md Results.
