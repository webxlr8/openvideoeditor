# Architecture plan and roadmap

Status: decided 2026-11-11. Supersedes the earlier FilmCraft-fork plan and the Electron draft (both removed).
Work is planned and tracked with OpenSpec: `openspec/changes/<change>/` (proposal, specs, design, tasks).

## 1. Scope of v1

A feature-for-feature desktop rebuild of the reference editor (docs/01-reference-teardown.md, docs/spec/*):
home and templates, projects, editor (library, layout, audio, captions, voice, text, transitions, effects panels,
timeline, viewer, properties), chat agent, CLI, MCP, export, settings. Then production-house features (plan pending).

Decided with the owner:
- Platforms: macOS 14+ Apple silicon, Windows 11 x64.
- Agents: bring-your-own only (Anthropic API key, or the user's Claude Code / Codex). No accounts, proxy or credits in v1.
- Motion graphics: agent writes JSX (same contract as the reference), rendered natively (see 3.4).

Not in v1: hosted agent, sign-in, plans, credits, cloud sync, web player, Intel Mac, Windows on ARM, Linux, 3D
(three.js) graphics, OpenFX/VST plugins.

## 2. Process layout

```
Tauri app process (Rust)
  ├─ UI: system web view (WKWebView / WebView2), React 19 + TypeScript. Panels, timeline, chat, settings.
  │      Typed commands/events generated from Rust types (one contract, no hand-written duplicates).
  ├─ Viewer: native wgpu surface (Metal / D3D12) placed over the viewer rectangle; owns playback, overlays, handles.
  ├─ core: document model, projection printer/parser, merge, revisions, verify      (pure Rust, no I/O deps)
  ├─ media: probe, decode (hardware first), proxies, thumbnails, waveforms, caches  (FFmpeg LGPL, dynamic)
  ├─ render: compositor shared by preview, `look` and export; audio mixer; encoder (hardware first)
  ├─ graphics: code-item runtime (QuickJS-NG sandbox → node tree → Takumi layout/paint → texture), Lottie (ThorVG)
  ├─ agent: Anthropic Messages client, Claude Code / Codex launchers, app MCP server (UI tools)
  └─ control socket (local, token-authenticated) ←── CLI binary (same core crates) ←── agents / scripts / MCP
```

Rules:
- One document pipeline: GUI, CLI, MCP and agents all produce ops that go through `apply` (parse → diff → rebase →
  revision). Nothing else writes project.json.
- Preview, `look` and export use the same compositor code and the same frame timing, so what the agent inspects is
  what gets exported (golden-frame tests enforce it).
- Heavy work (decode, render, export, transcription, proxies) runs off the UI thread with cancellation and progress.
- The core crates have no UI or platform dependency; they build and test headless on CI for both OSes.

## 3. Key decisions and alternatives

### 3.1 Shell: Tauri 2 (2.12.x)
System web view keeps the installer small; the reference app's UI is web-shaped (chat, cards, rich panels), which web
tech does best. Alternatives: Electron (bundles Chromium, ~300 MB, the weight we are removing); egui / Iced / Slint /
GPUI (native but slow to build a rich, polished UI; text/IME edge cases); full FilmCraft fork (a Premiere-class engine,
alpha, far larger than this product needs — individual MIT/Apache crates may still be reused, e.g. interchange, time).

### 3.2 Viewer: native wgpu surface composited with the web view (risk R1)
Decoded frames never travel through the web view. Preferred layering: the native surface sits beneath the web view and
the web view is transparent over the viewer rectangle, so web menus, dialogs and tooltips draw over the video.
macOS: CAMetalLayer-backed view below a transparent WKWebView. Windows: wgpu surface on the host window (or a
DirectComposition visual) beneath a WebView2 with a transparent background. Viewer overlays (safe areas,
transform/crop handles, caption edit outline) are drawn natively. Ordered fallbacks if the preferred layering fails on
Windows: (a) native child window above the web view, with web popups that overlap the viewer shown in a separate
small web view layer; (b) render in Rust and deliver frames to the web view through shared memory + a canvas (adds
latency, still hardware-decoded). Change 1 measures all candidates and picks by numbers.

### 3.3 Media: FFmpeg 9.x, LGPL-only build, dynamically linked
Proven decoder coverage (ProRes, DNxHR, HEVC 10-bit, MXF, …) and hardware paths on both OSes. No x264/x265 (GPL);
software H.264 encode via OpenH264 (BSD) or the platform encoder; hardware encoders preferred. Alternatives: GStreamer
(heavier, harder to ship on Windows), platform APIs only (two codebases, fewer formats), own Rust codecs (years).

### 3.4 Motion graphics: JSX → native renderer (risk R2)
Agent and templates write `<code>` items as a pure JSX function of `{time, width, height, assets}` (reference
contract). The source is transpiled (oxc) and run in a sandboxed QuickJS-NG context with no file, network or timer
access and a CPU/memory budget per frame; it returns an element tree (HTML elements + CSS/Tailwind subset). Takumi
(Apache-2.0/MIT; Chromium-ported block/flex/grid/inline layout, text shaping, gradients, masks, filters, blend modes,
keyframes) lays out and paints it; the result is uploaded as a texture into the compositor. Lottie assets render
through ThorVG (dotlottie-rs, MIT). Same output on both OSes, deterministic per frame, no browser.

Why not the alternatives: offscreen system web view (WKWebView/WebView2 capture is slow, not frame-deterministic and
renders differently per OS); bundled Chromium/CEF (+150 MB); Servo (full engine, MPL-2.0, heavy build; kept as a
possible later path for 3D); declarative JSON format (models write JSX/HTML/CSS far better; loses the reference's
expressive power). Limits we accept: no three.js/WebGL in v1; CSS limited to Takumi's subset — `check` rejects
unsupported features with a clear diagnostic and the agent docs list what is supported. Gate: change 4.

### 3.5 Timeline projection: literal TSX, parse-only
Same contract as the reference (docs/spec/document-model.md). Parsed with oxc; never executed (scripts via
`apply --eval` run in the sandbox, not the app). Item-level 3-way merge with property-based tests before any UI.

### 3.6 Agents: bring-your-own
Anthropic Messages API client written for this app (no official Rust SDK; community crates are early or archived):
streaming, tool loop, prompt caching, steering, per-turn token/time budgets, resumable sessions. Claude Code and
Codex launched as child processes with the app's MCP server and CLI on PATH (contracts in docs/spec/agents.md).
API keys live in the macOS Keychain / Windows Credential Manager, never in files or logs.

### 3.7 AI generation (video, image, voice, audio clean-up)
v1 is bring-your-own provider keys (same model as agents). Provider adapters behind one job interface with
`pending` placeholders, cost shown before spending, cancellation. Transcription is local (Whisper) so it works offline.

## 4. Roadmap (OpenSpec changes, in order)

Risk-first: the two technical gates (R1 viewer, R2 motion graphics) come before the bulk of the UI work.
Changes 1–3 are fully specified; later changes get their proposal/specs/design/tasks written just before they start,
so they build on what earlier changes actually delivered.

| # | Change | Capabilities | Depends on | Gate / exit |
|---|---|---|---|---|
| 1 | add-desktop-shell-and-native-preview | desktop-shell, media-playback | — | R1: 4K hardware playback in the Tauri window on both OSes, measured |
| 2 | add-project-document | project-document | — | Crash-safe bundles, revisions, undo; fault-injection tests |
| 3 | add-timeline-projection | timeline-projection | 2 | Round-trip + merge property tests; CLI print/check/apply/verify/history/undo |
| 4 | add-motion-graphics-runtime | motion-graphics | 3 | R2: frame budget met at 1080p/4K, sandbox escape tests |
| 5 | add-media-library | media-library | 1, 2 | Import/probe/proxies/thumbnails/waveforms/relink on the real-media matrix |
| 6 | add-compositor-and-export | render-engine, export | 1, 3, 4, 5 | Golden frames; export vs reference SSIM ≥ 0.99; hardware encode both OSes |
| 7 | add-editor-workspace | editor-ui | 1, 3, 5, 6 | Tabs, Home, editor, timeline, viewer tools, properties, shortcuts (5 presets) |
| 8 | add-agent-cli-and-mcp | agent-tools | 3, 6 | CLI command tree + control socket + MCP tools per docs/spec |
| 9 | add-agent-runtime | agent-runtime | 7, 8 | BYO agents complete a template end to end; budgets enforced |
| 10 | add-captions-and-transcription | captions, transcription | 6, 7 | Offline word-level transcription; 21 caption styles |
| 11 | add-text-and-animations | text-graphics | 4, 6, 7 | Text items, 16 animations, font library (open licences) |
| 12 | add-effects-transitions-layouts-color | effects, transitions, layouts, color | 6, 7 | Effect/transition/layout/LUT sets with parameter contracts from docs/spec/presets.md |
| 13 | add-templates | templates | 9, 11, 12 | Template format, catalog, intake forms; our own starter set |
| 14 | add-ai-generation | generation | 9 | BYO provider keys; pending placeholders; spend confirmation |
| 15 | add-interchange | interchange | 6 | FCPXML / FCP7 XML / EDL / OTIO round-trip into Premiere and Resolve |
| 16 | add-distribution | distribution | 1 | Signed + notarised DMG, signed MSIX/MSI, delta updates with rollback, crash reports, telemetry opt-out |
| 17+ | production-house changes | (from the owner's plan) | — | — |

Changes 1 and 2 can run in parallel. Change 16 starts early in parallel (signing and update plumbing are needed for
beta testers) and finishes last.

## 5. Engineering standards (apply to every change)

- CI on macOS (Apple silicon) and Windows 11 x64 runners for every PR: fmt, clippy `-D warnings`, tests, cargo-deny
  (licences, advisories, duplicate versions), UI tests, build artefacts.
- Tests per layer: unit; property-based for the document core; golden images for rendering; real-media matrix for
  decode/export; fault injection (kill during save, full disk, permission denied) for persistence; real-app UI tests
  driving the built app on both OSes.
- Performance budgets checked on fixed hardware: cold start, idle RAM, seek latency, dropped frames, export speed.
  A budget regression fails the build.
- No panic crosses an IPC or FFI boundary; every error reaching the user has a plain-language message and a log entry.
- Versions pinned; dependency updates via a weekly bot with the full test suite.

## 6. What the owner provides

- Windows 11 x64 test machine(s): one NVIDIA, one AMD or Intel GPU (or cloud GPU instances).
- Apple Developer account (signing + notarisation) and a Windows code-signing certificate (change 16).
- Test footage from real productions (camera originals, long interviews, multicam) for the media matrix.
- Product name and brand (before change 16), and the production-house plan (changes 17+).
