# Design: add-agent-skills-and-connectors

## Context

See proposal.md (Why). Builds on unify-agent-tool-layer (the tool registry and surfaces) and add-local-agents-and-onboarding
(agent detection, Connect). Contracts: docs/spec/agents.md, docs/spec/mcp-tools.md. Requirements: specs/agent-skills,
specs/agent-connectors.

Current state:
- `ove skills install` writes one `openvideoeditor/SKILL.md` (`crates/ove-cli/src/docs.rs`).
- The in-app prompt is the fixed `SYSTEM_PROMPT` in `crates/ove-agent/src/prompt.rs`.
- `McpSetup::Command` connects Claude Code only (`crates/ove-agents/src/lib.rs`).

## Goals / Non-Goals

**Goals:**
- Skills that every agent surface can read, at no cost to the prompt cache.
- Installing and updating skills from a catalog is safe by default: hash-checked, never silent, never overwrites a
  person's edits.
- One-step connection for the three most used local agent clients on both OSes.

**Non-Goals:**
- Running code from skills. Skills are text only.
- A marketplace, ratings or accounts.

## Decisions

### D1. Skill store layout
Skills live in `<library>/.skills/<id>/SKILL.md`. The frontmatter holds `name` and `description`, and optionally
`version` and `source`. The source is one of: builtin, catalog URL, local, or drafted. A ledger
`<library>/.skills/ledger.json` records, per skill, the installed SHA-256 and the catalog entry it came from.

Alternatives:
- Keep skills inside each project bundle. Rejected: skills are reused across projects.
- Keep skills in OS config folders. Rejected: the library folder is already where the person keeps work, and it is
  easy to back up.

### D2. Skill index in the prompt without breaking the cache
The system prompt is sent as two blocks:
1. The fixed `SYSTEM_PROMPT`, with a cache breakpoint.
2. The skill index, with its own breakpoint. The index is one line per skill: id, then a description cut at 200
   characters.

When skills change, only block 2 misses the cache. The agent loads a skill body with `read_skill`.

Alternatives:
- Inline all skill bodies. Rejected: it costs tokens on every turn.
- Leave the index out and rely on `list_skills`. Rejected: agents rarely call a tool for something they do not know
  exists.

### D3. Skills as MCP prompts and resources
`prompts/list` returns one prompt per skill, with the description. `prompts/get` returns the body as a user message.
`resources/list` exposes `ove://skills/<id>`. `list_skills` and `read_skill` are also tools, because many clients do
not show prompts.

Alternative: tools only. Rejected: clients that support prompts show them as slash commands, which people find useful.

### D4. Catalog with full-hash pins and visible updates
The catalog is a JSON document at a configurable URL, with entries `{id, name, description, sha256, path}`. Bodies are
fetched from that location with HTTPS and checked against `sha256`. A body that does not match is refused.

Updates follow these rules:
- When the catalog hash differs from the ledger hash, and the local file still matches the ledger hash, the app
  offers an update and shows a diff.
- The update is installed only when the person accepts it.
- A local file that the person modified is never overwritten. It is shown as "modified" and the update is offered as
  a separate copy.

Alternatives:
- Silent automatic updates. Rejected: text the person did not review would change agent behaviour.
- A short 12-character hash prefix. Rejected: too weak for content pinning.

### D5. Export to other agents
Export copies a skill into each detected agent's skills folder, under the name `ove-<id>`. The folders are those that
`ove-agents` knows: `~/.claude/skills`, `~/.codex/skills` and `~/.cursor/skills`. An exported copy that the person later
changed in that folder is not overwritten on re-export unless they confirm.

Alternative: symlinks. Rejected: Windows symlinks need developer mode or admin rights.

### D6. Drafting a skill from a project
`ove skills draft --from-project <p> [--with-chat]` builds a context pack:
- the final `timeline.tsx`;
- the media table;
- one summary line per revision, grouped by actor;
- the chat transcript, if included;
- the caption, effect and colour presets in use.

The pack is sent with our own drafting instructions to the configured assistant. The result has to answer two
questions:
1. How to rebuild this edit from the same media with no prior context.
2. How to apply the same style to new media.

The draft opens in the Skills pane editor. The person reviews and saves it.

In-app, the same drafting is available as a starter prompt and through `manage_skills`.

Alternative: rebuild the edit from screenshots. Rejected: we have exact data in revisions and in the TSX.

### D7. Starter skills
3 skills, written by us:
- **Caption styling:** choose one of our 21 presets, and verify the result with a frame.
- **Colour matching:** inspect, normalise, match, then build the look. This uses `inspect_color` once that tool
  exists; until then it uses frames and stats.
- **Interview cutting:** remove silences and fillers, keep sentence ends, and check with frames.

They are marked builtin and can be updated only by an app release.

### D8. Claude Desktop extension package
`tools/package.sh` builds `openvideoeditor.mcpb` per platform. The manifest uses the binary server type and points at
the bundled `ove` with the argument `mcp`. It lists darwin and win32. Settings → Agents → Connect opens the package with
the OS handler, so Claude Desktop installs it.

Alternative: a Node shim that forwards stdio to an HTTP server. Rejected: it needs an extra runtime and a fixed port,
and we have no HTTP transport.

### D9. Cursor install link
A `cursor://` install link carries the configuration `{command: <absolute ove path>, args: ["mcp"]}`, base64-encoded.
The link format is stored as data in `ove-agents`. A copyable JSON snippet for `~/.cursor/mcp.json` is always shown next
to it.

## Risks / Trade-offs

- **The catalog repository is compromised.** Mitigation: hash pins, HTTPS only, text-only content, updates approved by
  the person, and the source of each skill shown. Detection: tests that refuse tampered bodies and a hash mismatch.
- **Prompt injection through skill text.** Mitigation: skills cannot run code. The existing agent safety layers
  (add-local-agents-and-onboarding D6) still apply. The UI shows the source. Detection: a review checklist for the 3
  builtin skills, and a lint that flags tool names in a skill body that do not exist.
- **Claude Desktop refuses unsigned binaries.** Mitigation: the manual JSON configuration as fallback. Signing comes
  with add-distribution. Detection: the integration check on both OSes.
- **The Cursor link format changes.** Mitigation: the format is data plus a manual fallback. Detection: the integration
  check per release.
- **Many skills grow the index block.** Mitigation: cap the index at 60 skills, and show a warning in Settings above
  that. Detection: a unit test for the cap.

## Migration Plan

The existing `openvideoeditor/SKILL.md` written by `ove skills install` is still written. It now lists `read_skill`.
Existing installs keep working.

## Open Questions

- Waves proceed in master-plan order. This change is wave 2 (default).
- The community skill catalog URL is configurable and empty by default until the owner creates the public repository.
  3 original starter skills ship.
- Models and runtimes download on first use, with explicit consent above 50 MB. This change adds none.
- DirectML is off by default (CPU fallback) pending a licence ruling. Not used here.
- SVT-AV1 is excluded pending a ruling. Not used here.
- Remote model catalog: an embedded copy plus an optional signed remote URL setting; host to be decided. Not used here.

## Results

Measured on Windows 11 (x64), debug test builds.

### Group 1: skill store and reading skills

- Store: `crates/ove-tools/src/skills/` (the store, index, MCP prompt and resource shapes). The 3 builtin skills are
  compiled in and never written to the store folder; a store folder with a builtin id is listed as invalid. Folders
  that are links are skipped, so a skill cannot point an agent at another file.
- Skill text that agents read starts with a line naming its source and saying it does not change which tools the
  agent may use. Front-matter keys other than `name`, `description`, `version` and `source` are ignored.
- Fixed prompt bytes: identical with 0 and 20 skills, and across 10 turns with a skill installed before turn 6
  (`crates/ove-agent/tests/skills.rs`).
- Cache reuse, modelled: the mock Messages server recorded 10 requests with 20 skills installed (23 index lines with the
  builtin skills) and one skill installed before turn 6. Counting the estimated input tokens that repeat the previous
  request from the start (tools, fixed prompt, index, then messages), 98.7 % of the input of requests 2 to 10 could be
  served from the prompt cache. After the install, the tools and the fixed prompt still repeat; the index and the
  history after it miss once. A live measurement of `cache_read_input_tokens` needs a real API key and is open
  (task 1.3).
- `prompts/list` and `resources/list` over `ove mcp` are pinned by `crates/ove-cli/tests/golden/skills-*.json` and
  match the store (4 valid skills in the fixture: 3 builtin, 1 local; the invalid one is left out).
