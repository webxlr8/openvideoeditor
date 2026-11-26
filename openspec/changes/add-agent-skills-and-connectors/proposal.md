# Proposal: add-agent-skills-and-connectors

## Why

Agents edit better with playbooks. A playbook tells the agent how to grade shots, how to cut a podcast, or how to style
captions without making them ugly. Today the playbook support is thin:

- `ove skills install` writes one guide skill into the Claude Code and Codex folders.
- The in-app assistant's system prompt is fixed text.
- MCP `prompts/list` returns an empty list.
- People cannot add, share or update playbooks.
- People cannot turn a finished edit into a playbook that another agent can repeat.

Connecting an agent is also harder than it should be:

- Connect is wired for Claude Code only.
- Claude Desktop and Cursor users must edit JSON by hand.

## What Changes

**Skill store and how agents read skills**
- A skill store in the library folder holds one folder per skill, with a `SKILL.md` that starts with name and
  description frontmatter.
- The in-app system prompt gets a one-line index of the installed skills, in its own cached block. The fixed prompt
  text stays byte-stable.
- New tools `list_skills` and `read_skill` on every agent surface, through the unify-agent-tool-layer registry.
- Skills are served as MCP prompts and as resources (`ove://skills/<id>`).
- The in-app assistant gets `manage_skills` (create, update, remove), which writes to the skill store. Skills are not
  project documents.

**Community catalog**
- The catalog address is configurable. Each catalog entry pins the full SHA-256 of the skill file.
- An update to a skill is installed only when the local copy is unmodified, and only after the person sees the change
  and accepts it.
- Tampered content is refused.

**Moving skills between places and making new ones**
- Any skill can be exported into the skill folders of the local agents that we detect: Claude Code, Codex and Cursor.
  Exported skills get a name prefix.
- `ove skills draft --from-project <p>` collects the material for a new skill and drafts a reusable playbook from it:
  - the final `timeline.tsx`;
  - revision summaries by actor;
  - the chat transcript;
  - the styles and presets used.
- The person reviews the draft before it is saved.
- 3 original starter skills ship in the app: caption styling, colour matching, and interview cutting.

**Connectors**
- A Claude Desktop extension package (`.mcpb`) that runs the bundled `ove mcp` binary on macOS and Windows. It needs no
  other runtime.
- A Cursor install link built from the local `ove` path.
- Claude Desktop and Cursor entries in Settings → Agents → Connect, next to Claude Code, with a copyable manual
  configuration as fallback.

**Out of scope:**
- An HTTP MCP transport.
- Hosting a catalog service.
- Writing catalog content beyond the 3 starter skills.
- Code signing of the bundled binary. That belongs to the distribution change and its owner certificates.
- Chat `/` menu and mentions (add-agent-chat-context-and-activity).

**References:**
- docs/spec/agents.md: agent providers, launch contracts, prompt rules.
- docs/spec/mcp-tools.md: prompts and resources.
- docs/spec/cli.md: `skills`, `agents`.

## Capabilities

### New Capabilities
- `agent-skills`: the skill store, the skill index and how agents read skills on all surfaces, skills as MCP prompts and
  resources, in-app skill management, the hash-pinned community catalog with visible updates, export to other agents,
  drafting a skill from a project, and starter skills.
- `agent-connectors`: one-step connection of Claude Desktop (extension package) and Cursor (install link), next to the
  existing Claude Code connection, with a manual fallback.

### Modified Capabilities
- None.

## Impact

- New module `ove-tools::skills`, plus store code in `crates/ove-store` (library-level, not in bundles).
- `crates/ove-agent/src/prompt.rs` adds a second system block.
- `crates/ove-cli` adds `skills list|install|update|export|draft`.
- `crates/ove-agents` adds a deeplink connection kind and Claude Desktop detection.
- `tools/package.sh` builds the `.mcpb` package.
- The UI adds a Skills pane in Settings, and Claude Desktop and Cursor rows in `ui/src/agents/AgentsSection.tsx`.
- **Depends on:** unify-agent-tool-layer (registry, read tools).
- **Signing of the packaged binary:** add-distribution.
