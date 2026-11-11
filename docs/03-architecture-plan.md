# Architecture plan — Rust, macOS + Windows

Goal: an agent-native professional video editor for production houses. Native on macOS (Apple silicon + Intel) and
Windows (x64 + arm64), fast, light, stable. Built on proven open-source parts so we ship quickly without
re-inventing an NLE engine.

All facts below were checked on 2026-11-11 against the repositories (GitHub API: licence, stars, last push, CI runs,
Cargo manifests) and the projects' own docs.

---------------------------------------------------------------------------------------------------------------------
## 1. Decisions

### 1.1 Engine: fork FilmCraft (storytold/filmcraft)

Pure-Rust NLE, MIT OR Apache-2.0, ~18k stars, release v0.19.0 (2026-11-09), CI green on main, macOS / Windows / Linux
builds (signed + notarised DMG, MSI for x64 / arm64 / x86).

What it gives us on day one:
- Headless engine with strict crate layering; every action is a command (650+) with id, params, `enabled()` reason.
- Exact time: integer ticks, rational frame rates, timecode NDF/DF, feet+frames.
- Copy-on-write project snapshots; undo stack; lock-free readers for playback/export.
- wgpu compositor (Metal, D3D12, Vulkan) tested against a CPU reference renderer (golden images).
- Edit algebra (insert, overwrite, razor, ripple, roll, slip, slide, rate stretch, text-based editing), multicam.
- Codecs/containers in Rust: H.264, HEVC, ProRes, DNxHD/HR, AV1, VP8/9, MPEG-2, APV, DNG, AAC, AC-3, Opus, FLAC; MP4/MOV,
  MKV, MXF OP1a/OP-Atom, MPEG-TS/PS, Ogg, image sequences, BWF.
- Hardware decode: VideoToolbox (macOS), Media Foundation/D3D11 (Windows); hybrid fallback to software mid-stream.
  Hardware encode: VideoToolbox H.264/HEVC, NVENC H.264/HEVC (incl. HDR10).
- Interchange: EDL, FCP7 XML, FCPXML, OTIO, AAF, OMF, ALE — essential for production houses.
- Scopes (waveform, parade, histogram, vectorscope), BS.1770 / EBU R128 loudness, captions.
- Local speech-to-text (Whisper on candle, word timestamps) and local TTS (Kokoro).
- Agent access: MCP server via `rmcp` (official Rust MCP SDK) + JSON-lines control channel + CLI.
- Tests: unit, proptest, ffmpeg-oracle conformance (bit-exact where specified), fuzz/robustness, golden images,
  benchmarks; `cargo xtask ci` gates (fmt, clippy, tests, layering, assets).
- Dependencies permissive (egui 0.37, wgpu, rmcp, tokio, craft-libs Apache-2.0); no GPL/LGPL code.

Known risks (from its own docs) and how we handle them:

| Risk | Mitigation |
|---|---|
| Very young (created 2026-09-30), alpha, self-rated ~56% "ready for real work", 2,125 open issues | Phase 0 gate with hard pass criteria before we commit; pin a release; upstream our fixes to keep the fork thin |
| Own codecs are bit-exact on conformance streams but less tested on real camera files; Windows less tested than macOS | Real-media test matrix in Phase 0; optional FFmpeg decode backend behind the decoder registry (LGPL build, dynamically linked) if the gate shows gaps |
| No zero-copy GPU upload yet; colour grading, keys, masks on CPU | Performance track after the gate: zero-copy VideoToolbox/D3D11 → wgpu, move grading to WGSL |
| H.264/HEVC 10-bit 4:2:2 hardware decode missing on Windows | Software or FFmpeg path; document supported camera formats |
| No OpenFX / VST3 / AU hosting | Not needed for v1; revisit with the production-house plan |

### 1.2 UI: keep egui (FilmCraft's `ui-egui`), restyled to our design

One native GPU window, no web view, no second rendering stack: lightest footprint, lowest latency for the viewer
and timeline, and the UI crate is already integrated and tested with the engine (docking, panels, timeline, monitors,
playback, keyboard editor). egui is used in production desktop tools (e.g. Rerun).

Fallback if Phase 0 finds a UI requirement egui cannot meet: Slint (wgpu 30 texture/underlay integration documented;
royalty-free licence for proprietary desktop apps requires an attribution badge).

Rejected, with reasons:
- Tauri 2 + web UI: light and fast to style, but the 4K viewer must be a native wgpu surface layered with the web view;
  on Windows this needs its own child HWND and adds frame latency (tauri discussion #11944). Two UI stacks = more bugs.
- GPUI: Zed has publicly slowed GPUI framework work; OpenCut's GPUI desktop rewrite is ~38 KB of Rust today.
- Iced 0.14: its own FAQ says text editing still has edge cases; smaller pro-app track record.
- Electron: the weight we are trying to remove.

### 1.3 What we build on top (our product)

1. Document projection (the agent-native core): the timeline as a literal, parse-only text file with a revision header,
   item ids, item-level 3-way merge onto the live document, `pending` placeholders, diagnostics, revisions/undo.
   Parser: `oxc_parser` (MIT, JSX/TSX). Maps onto FilmCraft's project model and command layer.
2. Agent runtime:
   - Hosted agent: our own thin typed client for the Anthropic Messages API (no official Rust SDK exists; community
     crates are early or archived) with tool loop, streaming, steering, turn budgets (tokens/time), resumable turns.
   - BYO agents: launch the user's Claude Code / Codex CLIs against the project folder.
   - UI tools over MCP (`rmcp`): ask_user, pickers, confirm-spend cards; the agent parks until answered.
   - Structured intake forms for templates before the first agent turn.
3. CLI: extend `filmcraft-cli` (renamed) with our verbs: print/check/apply/verify/history/undo/look.
4. Templates (recipes): template folder format (instructions, structure, slots, locked graphics), catalog, installer.
5. Cloud services (later phase, only what the product needs): Rust (Axum) API, Postgres, S3-compatible object storage,
   job queue + GPU workers (generation, mattes, heavy transcription), OAuth + device flow, plans/credits, update feed.
6. Platform: crash reporting, signed/notarised builds, delta auto-update with rollback, telemetry opt-out.

---------------------------------------------------------------------------------------------------------------------
## 2. Phases and exit criteria

**P0 — Evaluation gate (≈2 weeks). No product code until this passes.**
- Build v0.19.0 on: Mac (Apple silicon), Mac (Intel), Windows x64 with NVIDIA, AMD and Intel iGPU, Windows arm64.
- Run `cargo xtask ci` with `FILMCRAFT_REQUIRE_ORACLES=1` on Mac and Windows.
- Real-media matrix (our own footage + public camera samples): ProRes 422/HQ/4444, DNxHR in MXF, XAVC / HEVC 10-bit 4:2:2,
  iPhone HEVC HDR, Canon/Sony/Panasonic MP4, 4K60 H.264, multicam (4 angles), long-GOP 1 h interview, 96 kHz audio.
- Measure and record: import success rate, 4K playback dropped frames, scrub latency, export correctness vs ffmpeg
  (SSIM/PSNR), interchange round-trip into Premiere / Resolve (XML, FCPXML, AAF), 4-hour soak crash count, RAM, install
  size, cold start.
- Pass thresholds (agree before testing): ≥98% of matrix files import and play; 4K H.264/HEVC/ProRes play at full res
  without drops on target hardware; export SSIM ≥ 0.99 vs reference; zero crashes in soak; interchange opens cleanly in
  Premiere and Resolve.
- Outcomes: GO (fork as is) · GO+ (fork + FFmpeg decode backend for the failing formats) · PLAN B (see §3).

**P1 — Fork foundation.** Rename and rebrand (FilmCraft brand assets are not licensed for reuse), keep notices; CI on
macOS + Windows runners; crash reporting; signing/notarisation; update channel (beta/stable).
Exit: signed nightly builds install and update on both OSes.

**P2 — Document projection + merge.** Design spec first; property tests (round-trip, concurrent edit pairs, id
stability, placeholder lifecycle) before any UI. Exit: GUI and text-file edits on the same project at once with zero
lost changes, verified in the real app.

**P3 — Agent layer.** Hosted + BYO agents, UI tools, intake forms, turn budgets, `look`/`verify` for the agent.
Exit: an agent completes a template end to end with no manual fix; token use per run measured and capped.

**P4 — Product UI.** Home, projects, editor chrome, chat panel, settings, onboarding — restyled egui to our design.
Exit: every screen in the screen inventory works on both OSes; real-app UI tests.

**P5 — Production-house features.** From your plan (expected candidates: shared storage, proxies, review/approval,
deliverables, AAF/XML round-trip to finishing tools).

**P6 — Cloud, accounts, billing.** Only after the local product works.

**P7 — Beta hardening.** Field testing with production houses, soak tests, performance track (zero-copy upload, GPU
grading).

---------------------------------------------------------------------------------------------------------------------
## 3. Plan B (if P0 fails)

Build the engine from mature parts and reuse FilmCraft crates that pass individually (they are separate crates):
FFmpeg via `ffmpeg-the-third` (active fork supporting FFmpeg 5.1–9.0; `ffmpeg-next` is maintenance-only) with
VideoToolbox / D3D11VA / NVDEC hardware decode, wgpu compositor, `cpal` audio, plus FilmCraft's `time`, `edit`,
`interchange`, `mxf`, `audio-dsp`, `scopes`, `captions`. Same UI and product layers as above. Slower to first release.

---------------------------------------------------------------------------------------------------------------------
## 4. Candidates considered and excluded

| Project | Why not |
|---|---|
| OpenCut (MIT, ~98k stars) | TypeScript web app; Rust/GPUI rewrite is early (~38 KB Rust) |
| Concat (~17k stars) | AGPL-3.0 |
| Cap (~25k stars) | Mostly AGPL (only camera/screen-capture crates MIT); screen recorder, not an NLE |
| Gyroflow (~10k stars) | GPL-3.0; stabilisation tool, not an NLE |
| Gausian native editor | Small, non-standard licence, last push 2026-07 |
| whisper-rs | Archived; FilmCraft's candle Whisper replaces it |
| rsmpeg | Last push 2025-08 |
| Kdenlive / Shotcut / MLT / Olive | C++ / GPL or stale; not Rust |
