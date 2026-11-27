# Tasks: add-agent-skills-and-connectors

## 1. Skill store and reading skills

- [x] 1.1 Implement the skill store (`<library>/.skills/<id>/SKILL.md`, frontmatter validation, ledger). Verify: unit
      tests for valid, invalid and missing-field skills.
- [x] 1.2 Add `list_skills` and `read_skill` to the tool registry for all surfaces. Verify: integration tests on the
      in-app and MCP surfaces, including the `skill.unknown` refusal.
- [ ] 1.3 Send the skill index as a second cached system block, capped at 60 skills. Verify:
      - a test that the fixed prompt bytes are identical with 0 and 20 skills;
      - a measured cache hit rate across 10 turns with 20 skills installed (record in Results).
      Open: the byte test passes and a modelled reuse is recorded in Results; the live hit rate needs a real API key.
- [x] 1.4 Serve skills as MCP prompts and resources. Verify: golden `prompts/list` and `resources/list` outputs that
      match the store.
- [x] 1.5 Document skills in the agent guide and `docs/cli/skills.md`. Verify: the doc examples run in the CLI
      integration tests.

## 2. Management, catalog and export

- [x] 2.1 Add the in-app `manage_skills` tool (create, update, remove; builtin protected). Verify: agent tests with a
      mock model, including the `skill.builtin` refusal.
- [x] 2.2 Implement catalog install with a full SHA-256 check, configurable URL, and no request when unset. Verify:
      - tests against a local catalog server for match, tamper and missing-entry cases;
      - a test that makes no network request when unset.
- [x] 2.3 Implement visible updates (diff view; install only on accept; modified local skill kept, update offered as a
      copy). Verify: unit tests for the ledger states, and a UI test of accepting an update.
- [x] 2.4 Implement export to detected agents' skill folders, with the `ove-` prefix and an overwrite confirmation.
      Verify: integration tests with temporary home folders on both OSes.
      Run on Windows 11 here; the same tests run on macOS in the CI matrix (not run by this worker).
- [x] 2.5 Build the Settings → Skills pane (Installed, Community, Local; editor; export menu). Verify: UI tests for
      list, edit, install and export.

## 3. Drafting and starter skills

- [x] 3.1 Implement `ove skills draft --from-project` (context pack, our own drafting instructions, review editor in the
      app). Verify:
      - a test with a mock assistant that the pack contains timeline, history and presets;
      - a test of the no-assistant path that saves the pack to a file.
- [x] 3.2 Write the 3 starter skills (caption styling, colour matching, interview cutting) as original text. Verify:
      - the lint passes (no unknown tool names);
      - an agent test with a mock model follows each skill on the sample project without tool errors.
- [ ] 3.3 Draft a skill from the sample project's finished cut and run it on the same media in a new project. Verify:
      `ove verify` diff of the two timelines within tolerance (record in Results).
      Open: drafting the skill needs a real assistant (API key); not run by this worker.

## 4. Connectors

- [ ] 4.1 Build the `.mcpb` package per platform in `tools/package.sh` (binary server, darwin and win32). Verify: the
      package manifest validates with the published schema in CI.
- [ ] 4.2 Add the Claude Desktop detection, Connect and status rows, and the manual configuration fallback. Verify: UI
      tests for the installed, not-installed and refused states.
- [ ] 4.3 Add the Cursor install link (connection kind with the link format as data), the configuration snippet, and
      status detection from `~/.cursor/mcp.json`. Verify: unit tests for link encoding and status parsing; UI test.

## 5. Integration checks

- [ ] 5.1 On built apps on macOS and Windows, install the extension in Claude Desktop and connect Cursor through its
      link. Each client lists the editor's tools and makes one edit that appears in the open project. Record the
      results.
- [ ] 5.2 On both OSes, install a catalog skill from a test catalog, export it to Claude Code, and invoke it there as a
      slash prompt. Record the results.
