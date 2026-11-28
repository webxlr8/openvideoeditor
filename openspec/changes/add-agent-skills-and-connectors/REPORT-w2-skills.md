# Worker report: add-agent-skills-and-connectors (branch azharudh33n/w2-skills)

Head: 1304da9, which contains azharudh33n/hydroid at b9ed6c0. Nothing pushed.

## Task groups

| Group | State | Commit |
|---|---|---|
| 1. Skill store and reading skills | Done except the live part of 1.3 | 7bc9888 |
| 2. Management, catalog and export | Done | 4577171 |
| 3. Drafting and starter skills | 3.1 and 3.2 done; 3.3 open | c8f5a9f |
| 4. Connectors | Done (signing open) | 48076d2 |
| 5. Integration checks | Open | — |
| Security review fix | Done | fa87819 |
| Merges of azharudh33n/hydroid | Done | cbb094c (11 conflicts), f0d5ca8 (9), 1304da9 (1) |

Measured numbers are in design.md under Results:
- Modelled prompt-cache reuse over 10 turns with 20 skills, with one skill installed before turn 6: 98.7 %.
- Test `.mcpb` package on Windows (debug `ove.exe` and 6 FFmpeg DLLs): 20.5 MB. After unpacking it with the official
  tool, `ove mcp` answered `initialize`, listed 48 tools and listed the 3 builtin skills as prompts.

## Final check: after the last merge, full property-test counts

The reduced property-test counts were unset (`unset OVE_PROPTEST_CASES OVE_PROP_CASES`) and
`OVE_ALLOW_MISSING_MEDIA_TOOLS=1` was set.
- `cargo fmt --all -- --check`: ok.
- `cargo clippy -p ove-tools -p ove-cli -p ove-desktop -p ove-agent -p ove-agents --all-targets -- -D warnings`: no
  warnings.
- `cargo test --no-fail-fast` on the same crates: 47 test binaries, all `test result: ok`, 317 passed, 0 failed.
- The second merge brought in more crates, so a wider run checked ove-tools, ove-cli, ove-render, ove-tasks,
  ove-store, ove-media, ove-generate, ove-control, ove-agent, ove-agents and ove-desktop:
  - 86 test binaries were ok and 1 failed: `ove-cli` `model_worker::image_tensor_round_trip_is_fast`.
  - The failure was the timing check `median <= 2.0 ms`, missed while the machine was under parallel load.
  - Run alone, that test passed 3 times out of 3.
- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok.
- `openspec validate add-agent-skills-and-connectors --strict`: valid.
- `node tools/mcpb/pack.mjs --self-test`: passed.
- UI: 37 files and 221 tests passed. Typecheck and lint are clean.
  - Under heavy cargo load, one UI timing test failed in 2 earlier runs. One of them was `SkimThumb` "each tile within
    16 ms". Every rerun passed.
  - `ove-desktop` `scrub::tests::scrub_latency_and_edits` failed once under load. It passed when run alone.
- Skipped: the HDR fixture test, which needs python3 (`OVE_ALLOW_MISSING_MEDIA_TOOLS=1`).

## Tasks left open
- **1.3, live cache-hit rate.** It needs a real Anthropic API key. The byte-identical test and the modelled number
  are done.
- **3.3.** Draft a skill from the sample cut and rebuild it. Needs a real assistant key.
- **5.1 and 5.2.** Need the built apps on macOS and Windows, with Claude Desktop, Cursor and Claude Code installed.
- **Signing the `.mcpb` binary.** Needs the owner's certificates; it belongs to add-distribution.
- **2.4 on macOS.** The tests ran on Windows only. They are cross-platform, and CI runs them on macOS.

## Decisions to review
- **Each `.mcpb` lists only its own platform.** D8 says the manifest lists darwin and win32. A package that holds
  the macOS binary cannot run on Windows, so each package lists only the platform it carries.
- **The package ships the FFmpeg libraries and the notices.** Without the DLLs the server does not start.
- **Catalog installs need the pin the person saw.** `ove skills install` needs `--yes --sha256 <pin>`, and Settings
  passes the previewed pin. This came out of the security review.
- **Exports never copy other front-matter keys.** Exported skills get fresh front matter: `name: ove-<id>` and the
  description. Keys such as `allowed-tools` therefore never reach the other agent.

## Edits outside my owned files
All additive:
- `apps/desktop/src-tauri/src/assistant.rs`: the skill store hook, the skill index for each turn, and
  `key_and_model`.
- `apps/desktop/src-tauri/src/agents.rs`: `ove_program` is now `pub(crate)`.
- `apps/desktop/src-tauri/src/lib.rs`: module and command registration.
- `apps/desktop/src-tauri/Cargo.toml`: `zip`, already in the lock file through ove-models.
- `apps/desktop/ui/src/Settings.tsx`: the Skills section and the Desktop apps rows.
- `apps/desktop/ui/src/bridge.ts`: spreads the two mock modules and adds their state.
- `crates/ove-agent/src/{agent.rs,tools.rs,lib.rs}`: the second system block, tool labels and `draft.rs`.
- `crates/ove-agents/src/lib.rs`: `SKILL_TARGETS` and `clients.rs`.
- `crates/ove-cli/src/{main.rs,mcp.rs}`: the `skills` subcommand, MCP prompts and resources, and the dependency on
  ove-agent.
- `crates/ove-tools/src/{ctx.rs,app.rs}`: the `skills()` accessor.
- `crates/ove-tools/src/registry.rs`: 3 tools added at the end.
- `crates/ove-tools/src/tools/docs.rs`: the skills topic.
- Goldens, `docs/cli/{agents,mcp,tools}.md`, `.github/workflows/ci.yml` (an .mcpb manifest step) and
  `tools/package.sh`.

## Security review summary
- The `security-review` skill reviewed the branch diff against azharudh33n/hydroid.
- No finding reached the reporting bar (confidence of 8 or more).
- One finding below the bar was fixed anyway in fa87819, because it broke the "person confirms each install"
  condition: an install did not check that the text was still the text the person had read.
- One note was left unchanged: an exported skill body is copied as written. The other agent's own permissions still
  apply, and export runs only when the person clicks it.
- Details are in design.md under Results.
