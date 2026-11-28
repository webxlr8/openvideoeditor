# Plan: from `dev` to 0.1.0-alpha.2

Status: draft plan, 2026-11-28. Reviewed by the CTO and CEO reviewer agents; both approved it with the conditions
below. No step in this plan has been pushed or published.

## 1. The `dev` branch

- `dev` starts from `main` (469bfa0, the 0.1.0-alpha.1 line) and merges `azharudh33n/hydroid` (d387d9a): waves 0–3
  of the post-v1 roadmap (`docs/03-architecture-plan.md` §4.1).
- `phase2-local-agents` (35 commits, includes `tasks-audit`) is not in `dev` yet. A trial merge into hydroid conflicts
  in 29 files. It is ported later in a separate branch, `port/phase2-onto-dev`, in topic groups:
  1. Decoder, allocator and packaging: mimalloc as the global allocator (a 4x slowdown on Windows without it), the
     hardware-decode parity test, the software-decoder fallback, decoder thread limits, MSVC FFmpeg builds,
     `tools/package.sh`, and library paths with spaces.
  2. The agent client protocol, only where hydroid's own clients do not already cover it.
  3. Work hydroid already supersedes is dropped: GPU effects, scopes, "Remove silences" and "Clean up voice". Where
     both branches have a feature, the wave's version is kept when it is that wave's exit check.
  The port passes when it fails nothing in section 2, the hardware-decode test passes, and the app e2e flow count
  does not go down.
- Port status (branch `port/phase2-onto-dev`, 2026-11-28):
  - **Ported (18 commits, cherry-picked with `-x`):** the hardware-decode test; mimalloc; native Windows packaging
    (`FFMPEG_OUT`, MSVC and DLL paths, library paths with spaces); FFmpeg's `av1` decoder for the hardware path and
    bounded decoder threads; the GPU device opened with the first frame (Home 257 → 55 MB working set); the
    timeline scrollbar gutter; the forced-panic e2e flow; the software-decoder fallback; diagnostics without a GPU;
    Settings → Library (choose another library folder); Settings → add the command line to PATH; caption styles
    drawn with the font library; motion templates laid out for vertical video. Conflicts were resolved keeping both
    sides. On OpenSpec `tasks.md` files dev's text was kept, because phase2 reorganised them; phase2's measured
    Results in `design.md` were kept.
  - **Dropped, superseded by hydroid:** the pinned `ove mcp --project/--turn` (hydroid's registry has the `Pinned`
    surface and `--turn`); "the built-in assistant sees the project" (registry `look` and frames); the transcript
    tool (`get_transcript`); "Remove silences" (transcript dead-air removal); scopes in the Colour tab and GPU effects
    for full-size pictures (the GPU compositor and scopes); "Clean up voice" (the audio-intelligence denoise, still in
    progress on `w3-audiointel`).
  - **Not ported yet: chat through an installed agent (Agent Client Protocol).** This is open task group 3 of
    `add-local-agents-and-onboarding` on dev. phase2 built it on its own `ove-agents` and `ove mcp`, and both were
    restructured on hydroid. It needs reimplementing on dev's architecture, using phase2's commits as the reference
    (993e9d3, b1d5946, cac7899, b78c0c6, d3a2189, 8835cd6, 83fe482, 39a4b79); a worker does it. It is not needed for
    0.1.0-alpha.2: the built-in assistant (your own Anthropic key) is the chat in that release. The CEO reviewer
    approved this, on three conditions:
    1. Nothing in alpha.2 offers chat through installed agents: the release branch removes any picker text or
       setting that promises it. The MCP Connect buttons stay; they let external agents edit through the app.
    2. The release notes say: "Chat through installed agents (Claude Code, Codex): not yet; coming next."
    3. Task group 3 stays open, and a worker starts on it right after the release branch is cut.
- `w3-audiointel` and `w4-interchange` are still in progress. They join `dev` after the release branch is cut, not
  before. The seven changes that have not started are out of scope for this release.

## 2. Build everything and test everything (Windows 11, this machine)

Run on `dev` after sourcing the env script, with the full property-test counts (no `OVE_PROPTEST_CASES`):

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features --no-fail-fast`; record the count of passing tests.
4. `cargo run -q -p ove-desktop --features bindings --bin export-bindings -- --check`
5. `cargo deny check`
6. `openspec validate --all --strict`
7. `apps/desktop`: `pnpm install --frozen-lockfile`, then `pnpm run typecheck`, `pnpm run lint`, `pnpm run test`
   and `pnpm run licenses`.
8. `node tools/mcpb/pack.mjs --self-test`
9. `pnpm tauri build --debug --no-bundle`, then `pnpm run e2e:app` (the built app with tauri-driver).
10. `OVE_REQUIRE_GPU=1 cargo test -p ove-gpu --test parity --test render_parity`
11. `tools/package.sh`; then measure the installer size (≤ 60 MB), idle RAM with one project open (≤ 250 MB) and
    cold start to Home (≤ 1.5 s).

These cannot run here and are owed by CI or the owner: the macOS build and VideoToolbox; the macOS and Linux CI jobs;
the FFmpeg rebuild; GPU reference hardware; real accounts and provider keys.

## 3. What can be released

Nothing is released today. The most that is honest is **0.1.0-alpha.2, Windows only, as a draft pre-release for
private testers**. A public alpha ships on both operating systems together. It waits until CI on macOS is approved, the
Mac hands-on rows of `docs/owner-checklist.md` are done, and signing is decided.

Gates before tagging:
- every step of section 2 passes, and CI passes on GitHub for the tagged commit;
- the FFmpeg rebuild (the shipped libraries cannot read the PNGs the generation tools create) is done, or it is listed
  as a known limit;
- the OpenH264 decision is made (the reviewers recommend leaving it out for now, because that can be undone), and
  `check-license.sh` and `tools/package.sh` pass after the rebuild;
- features not yet cleared are turned off or hidden: semantic search (SigLIP 2 licence), DirectML, SVT-AV1 and the
  remote model catalog;
- release check on a clean Windows 11 machine:
  - describe an edit, get a timeline, adjust it and export it;
  - recovery after a crash;
  - no network traffic except to providers the person has given a key for;
  - the three budgets in section 2.

The version stays 0.1.0 in `Cargo.toml` and `tauri.conf.json`. The tag is `v0.1.0-alpha.2`. The release notes go
in `docs/releases/0.1.0-alpha.2.md`, which `release.yml` requires. They must say:
- macOS has not been tested in the built app;
- the Windows installer is not signed, so SmartScreen warns, and there is no signed update;
- semantic search is not available yet;
- the H.264 export path follows from the OpenH264 decision;
- AI generation has not been tested with live provider keys;
- playback speed on the reference machines and transcript accuracy have not been measured;
- waves 3 and 4 are not finished; the open OpenSpec tasks are listed.

## 4. Steps that need the owner's explicit approval

None of these happens without the owner's approval:
- pushing `dev`; hydroid itself is not on origin yet;
- pushing a `v*` tag, which starts `release.yml` on paid macOS and Windows runners and is hard to take back;
- setting `OVE_RELEASE_MACOS=false` so that a Windows-only release can publish;
- publishing the draft GitHub release, or any upload;
- any push to `main`;
- the licence decisions on OpenH264, DirectML and SVT-AV1, and the approval for CI on macOS and Linux.

## 5. Order

1. Section 2 on `dev`; fix anything that fails, as commits on `dev`.
2. `port/phase2-onto-dev` (section 1); section 2 again.
3. The owner's decisions (section 4); the FFmpeg rebuild; licence and packaging checks.
4. Cut `release/0.1.0-alpha.2` from `dev`; turn off the features not cleared; write the release notes.
5. The release check on a clean Windows 11 machine.
6. With approval: push, tag, draft pre-release, testers.
7. Later: the public alpha on both operating systems.
