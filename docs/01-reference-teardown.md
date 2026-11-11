# Reference app teardown (macOS arm64 desktop build)

Source: static reading of the shipped desktop app (main, preload, renderer, worker), its bundled CLI and docs, a real
project bundle on disk, the running app, and the public docs/pricing. Anything marked (inferred) was not observed.

Product: AI video editor — "describe the edit, get a real timeline you can still adjust".

---------------------------------------------------------------------------------------------------------------------
## 1. The one idea that makes it work

The edit document is exposed as source code, and every editor (human, built-in agent, external agent, CLI, MCP) edits
the same document through the same diff/merge pipeline.

- Each project is a folder bundle `<Name>.<bundle>/` in `~/Movies/<App>/accounts/<sha256(account)>/`.
- `project.json` is canonical and never hand-edited.
- `timeline.tsx` is a *literal* TSX projection of the timeline, regenerated on every save. It is parsed, never executed.
  Editing and saving it applies the change (live in the open app within ~200 ms, as one undo step).
- Header of `timeline.tsx` carries: aspect, duration, track/item counts, a media table (names usable as `src`),
  `diagnostics` from the last apply, and `// app:rev <n>`.
- Apply = parse → diff against the revision it was printed from → item-level merge onto current doc. Items keep `id`s;
  items without an id are new; deleted items are removed; conflicts resolve `--prefer file|editor`.
- `pending` placeholders: agent places empty labelled items first ("place before you fill"), they shimmer on the
  timeline, block export, and `verify` counts them. Human sees the shape of the edit while the agent works.
- `.revs/` keeps revisions → `<cli> history`, `<cli> undo [rev]`.
- `timeline.d.ts` beside it gives the agent full typings. `AGENTS.md` in each bundle gives agent house rules
  (stay inside the bundle, media is read-only, scratch work in `.scratch/`, user-pasted files in `.context/`).
- Loops/helpers: write a script and `<cli> source apply <p> gen.tsx --eval` (runs in CLI process, same JSX factory).

Element set: `<timeline aspect>`, `<track kind=video|audio|caption|adjustment|layout>`, `<clip>`, `<image>`, `<audio>`,
`<text>`, `<caption>`, `<code>` (JSX motion graphic: pure function of {time,width,height,assets}, linted on apply),
`<adjustment>` (LUT/colour grade layer), `<shader>` (GLSL fragment shader over everything beneath, gets person matte).
Common attrs: id, name, start/end (timeline s), in/out (source s), transform{x,y,scale,rotation,flipX}, crop, opacity,
zIndex, blendMode, keyframeTracks, transitionIn/Out, effects, colorGrade, captionComposition{front|behind-subject}.
First track = top layer. Clips without a transform auto-reframe to the aspect.

---------------------------------------------------------------------------------------------------------------------
## 2. Process / runtime architecture

Electron app (Electron Forge, Vite, esbuild, TS). Fuses enabled, ASAR integrity on.

Main process (`dist/main.js`, 2.9 MB minified)
- Single typed RPC bridge. Preload exposes only `window.desktop = { platform, getPathForFile, send, on }` with three
  channels: `main:request`, `main:response`, `main:event`. All payloads are zod-validated. A `WH` router class maps
  channel names → handlers; it buffers requests until `markReady()`.
- 105 request handlers (see docs/spec/ipc.md), grouped (partial list):
  account (get, plan, plan-features, sign-in, sign-in-copy-link, sign-out), access:get (invite gating),
  library (call, choose, info, move, move-cancel, retry, reveal, status), project (patch, rename, reveal, suggest-name),
  media (relink, relink-folder, unregister), file (preview, reveal), reference (import, inspect),
  recipes (fetch, cached, remember), local-agent (status, send, install, logout, chat-skills, agent-advisory[-get]),
  agent-tools:install, agent-setup:done, skills:status, cli:status, control:reply, edit:native-command,
  export (log, sink-read), tabs (list, activate, close, move, peek, overlay, command), window (is-fullscreen,
  set-editor-state), update (check, status, dismiss, recall-get), telemetry (capture, defect, feedback, settings),
  feedback (submit, recording, recording-stop), welcome (get, seen), thanks (get, said), renderer-diagnostics:*.
- 11 events main→renderer: control:request, editor:get-more-credits, editor:set-aspect-ratio, feedback:recording-stop,
  home:recipe, home:section, menu:action, settings:open, shell:navigate, tabs:modifier, update:notice.
- Custom privileged schemes registered (standard, secure, fetch, CORS, stream) to serve renderer + media.
- `<app>://` URL scheme → opens a recipe on Home. `open-file` for bundles / `.<archive>`.
- Window: `titleBarStyle: hiddenInset`, custom traffic-light position, dark only.

Tabs
- Browser-style tab strip (`tabs.html`). Each tab = its own WebContentsView renderer (Home, Settings, one per project).
- Tabs can sleep (unloaded to save memory) and wake on any command. A hidden `/app/warm` route pre-warms a renderer.
- Only the front tab is audible. Agents act on background tabs without stealing focus.

Library worker (`dist/library-worker.js`, separate utility process)
- Owns the on-disk library: projects.list/summaries/posterCards/usage/freeSpace/read/write/trash/saveContextFile,
  media.locate, a key/value cache (get/put/delete/keys/clear), transcripts.ensure, mattes.ensure,
  audioProcessing.process, voiceovers.clone, timeline generation keys, LUTs (list/read/add).
- Library relocation (move library folder, verify copy, old one to Trash), network-drive refusal, APP_HOME override.
- Uses bundled ffmpeg 6.0 / ffprobe 4.4.1, yt-dlp (system or downloaded), derived caches per media hash:
  `cache/peaks`, `cache/meta`, `cache/waveforms`, plus `proxies/`.

Renderer (`dist/renderer`, React 19 SPA, shared with the web editor at <vendor>)
- Stack: React 19.2, react-router 7, zustand 5, TanStack Query 5 + Virtual, zod 4, motion 12, Tailwind 4, Geist +
  Inter fonts, Phosphor icons, sonner toasts, shiki + mermaid + KaTeX (chat rendering).
- Routes: /app/home/*, /app/projects, /app/settings, /app/editor/:id, /app/subscribe, /app/warm.
- Rendering: custom WebGL2 compositor; WebCodecs decode/encode; mediabunny (AAC encoder, AC3 decoder, ProRes);
  HEIC decode; GIF source. Workers: export.worker, frameEncoder.worker, waveform.worker, timer.worker.
- Services: IngestionService, ProxyService, ThumbnailService, WaveformService, VideoAnalysisService,
  face-detection-jobs, active-speaker-detection-jobs.
- `<code>` graphics runtime (`vendor/code-clip-runtime.js`, 2.6 MB): React + three.js + lottie inside, sandboxed.
- A separate embeddable player (`card/player/player.js`) used for share links and the in-chat player.

bundled CLI (`<cli>`) (`Resources/cli/cli.mjs`, run as `ELECTRON_RUN_AS_NODE=1 <App> cli.mjs`)
- Same library, works with the app closed. Talks to the running app over a control channel (control:request/reply).
- Commands: project, media, download, capture (headless Chrome screenshot), footage, music, sfx, audio, generate,
  voice, source (print/check/apply), verify, history, undo, app (context/open/seek/play/pause), screenshot, look
  (render composited frames/contact sheets from the live preview), export (local via app, or --cloud), code, doctor,
  docs, skills (install skill into Claude Code/Codex/Cursor), mcp (serve CLI as stdio MCP), auth, update.
- Exit codes are part of the agent contract: 3 = not signed in / app not running / no tab, 4 = out of credits.
- Docs shipped as markdown: `commands/*.md`, `techniques/*.md` (motion-graphics, synced-captions, color-grading,
  behind-subject compositing, 3d-scenes, shader presets), `skills/<app>/SKILL.md`.
- uv bundled to provision a pinned Python (numpy, opencv-headless, pillow, scipy) for agent scratch scripts.

Agents (four providers, enum claude | codex | hosted | gemini)
- built-in agent ("Cloud"): the bundled Claude Code native binary (`Resources/<agent>/claude`, 277 MB) driven through
  `@anthropic-ai/claude-agent-sdk`, model `claude-opus-5-5[1m]`, effort medium default. All ANTHROPIC_/OPENAI_/... env is
  stripped and replaced; LLM traffic goes to the vendor's own `/<agent-proxy>/v1/messages` proxy, billed as plan usage.
  Long-lived turn, steerable mid-turn, per-chat state dir `.desktop/<agent>/<hash(account)>/<hash(project)>/<hash(chat)>`.
- Claude Code / Codex on this Mac: uses the user's own Claude / ChatGPT sign-in (Settings › Agents). Needs paid plan.
- A Gemini-based provider: bundled, present in enums, not exposed in Settings in this build.
- In-app MCP server `app_ui` exposes only UI-interaction tools: `ask_user` (1–4 questions, 2–4 options,
  recommended first), `pick_voice` (2–4 voices with samples), `clone_voice` (record + clone card). The agent "parks"
  until the human answers in the chat panel. Everything else the agent does through `<cli>` in a shell.
- Chat stored per project: `agent/chats.json` + `agent/chats/<id>.jsonl` with typed records
  (user, session, assistant blocks incl. tool_use, tool-result, question, result with durationMs + threadUsage).
- Chat skills (toggleable "guardrails"): accessibility, brand-safe, compliance (`Resources/chat-skills/*/SKILL.md`).
- Same surface externally: remote MCP (`<vendor>/mcp`, OAuth, 37 tools on Claude endpoint), ChatGPT app,
  Claude connector, `<cli> mcp` locally.

Recipes (templates the agent executes)
- A recipe is a folder copied into the project: `recipe/RECIPE.md` (instructions to the agent), `structure.json`
  (length, voice, scenes with start/end, narration word budgets, media slots), `inputs.json` (subject, story, media),
  `templates/*.jsx` (all look and motion), `shaders/*.glsl`, `guide.jpg` (labelled key frames), `recipe.json`.
- The agent's job is narrowed to story + casting media into slots; it never writes graphics code for a recipe.
- Recipe catalog fetched from server (`recipes:fetch/cached`), with category, trending flag, aspect ratios, length,
  inputs, kit, samples, "usesGeneration", poster/preview/video URLs. "Remix a reference" = start from a video link.

Cloud backend (`<vendor-web>/api/v1/*`, auth = Clerk OAuth, device flow for CLI)
- me, auth/config, desktop/{access,activation,telemetry,update}, cloud-projects, cloud-time, uploads,
  transcriptions (speech-to-text is server-side "Media Service"), mattes (GPU subject cut-out), audio-processing
  (clean speech, remove music, stems), voices, voiceovers (ElevenLabs), generations + generations/models (video/image/
  music/sfx: Hailuo, Kling, Seedance, Veo, Wan...), sounds, footage/search (Pexels, Pixabay, Coverr, Wikimedia,
  Internet Archive, YouTube), youtube-imports, exports/{cloud,claim,<id>}, agent-turns/<id>, feedback(+uploads),
  plugin/previews/report. Assets on <vendor-cdn>.
- Telemetry: PostHog + Sentry; opt-out in Settings › Privacy.
- Updates: own update server (`/api/v1/desktop/update`), Squirrel.Mac, channels alpha/beta/stable.

---------------------------------------------------------------------------------------------------------------------
## 3. Screen inventory (every screen and what is on it)

.

3.1 First run / gating
- Sign in: "Continue with Google", Terms/Privacy, browser hand-off with "Copy link" and "Paste the code from your
  browser" (verification code).
- Invite gate: "This build goes out to a small group of testers… Ask for an invite / Check again / Sign in with a
  different account".
- Offline screen: an internet connection is required to set up.
- Welcome survey (3 steps, Back/Skip): How did you hear about us? (social, community, AI tools, friend, Product
  Hunt…) → What best describes you? (Founder, Marketer, Creator, Product, Engineer, Designer, Student) → What will you
  make? (Talking-head, Explainer, Product demo, Launch video, Vlog, Montage, Podcast clips, Ad/Promo, Something else).
- Agent consent: consent screen for using installed agents (switches cover installed agents only). Continue /
  Skip for now.
- Plan picker (Monthly/Annual, Free to get started on Mac).

3.2 Home (Dashboard tab)
- Prompt box ("Ask … to edit") with aspect picker (Landscape, Tall, Square, Classic), attach media / reference.
- "Start from a recipe" / "Make your first video" / "Remix trending videos" / "Upload a reference" (paste link:
  YouTube, Drive, TikTok… "Reading the link").
- Recipe browser: All / Short-form / Long-form, Length filter, search; recipe card → detail ("Customize this recipe
  with your media and details", You'll add…, Works best with…, Uses generative credits, Made for <aspect>, Try with
  sample media).
- Your projects (grid, rename, delete, multi-select), What's new (feature demos, collapsible), update banners
  ("Restart to update").
- Sections nav: Dashboard / Projects / Account / Settings.

3.3 Projects (/app/projects): searchable list, select all, rename (folder keeps name), delete → Trash.

3.4 Editor (/app/editor/:id) — four regions 
- Left rail (icon tabs): Library, Layout, Captions, Voice, Text, Transitions, Effects (+ Audio/Sounds).
  - Library: search project media (semantic: "object, action, place, visible text, or transcript phrase"), Add,
    cards with waveform/thumbnail + duration, preview with "About this clip" insights, add best segment / full clip,
    stock sources tabs (Pexels, Pixabay, Coverr, Wikimedia, Internet Archive, YouTube), drop to import.
  - Layout: layout presets (split, grid, hero, PIP) with slot assignments.
  - Captions: generate (language), regenerate, translate, presets tab (animated caption styles), transcript editor.
  - Voice: script → speech, voice library (filters: gender, age, accent, language), your voices, clone (record or
    upload 40 s+), advanced (stability, similarity, style, speaker boost, speed).
  - Text: presets and sections, fonts (featured/all), animations.
  - Transitions: presets, duration, direction, easing, interactive A/B scrub preview.
  - Effects: ~45 GLSL looks in groups Stylize / Distort / Backgrounds / Texture & finish / Color looks, each with
    named params; "Ask built-in agent to change this effect"; editable effect code.
  - Sounds: music library (genre, mood), SFX library (category), add to timeline.
- Centre: preview canvas (WebGL), fullscreen, frame export, crop mode.
- Bottom: transport (timecode, undo/redo, snapping, play, frame step), zoom slider, multi-track timeline, keyframe
  lanes, "Drop media to start", shimmering pending placeholders with progress, generation status cards.
- Right: chat panel — recipe chip, agent/model/effort picker (Cloud/Claude Code/Codex; low…max), Skills toggles,
  conversation history (named, searchable, new conversation), queued messages, "Worked for Xm Ys" expandable work log,
  copy / thumbs up / thumbs down ("What could be better?"), inline question cards (Skip / Back / Next), voice pick /
  clone cards, attachments ("They go into the message where they are. Nothing joins the project until the agent puts
  it there.").
- Properties panel (on selection): Basics, Transform (scale, rotation, position, crop, flip), Visual (opacity, corner
  radius, blur, blend modes), Fades, Playback (speed, reverse), Audio (volume), Animation presets (Fade, Blur In,
  Typewriter, Glitch, Rise Up, Drop In, Slide, Settle, Zoom, Pop, Focus Pop, Rotate…), Text styling (font, size,
  line height, spacing, stroke, shadow, container BG), Color tools (Adjust: exposure, contrast, highlights, shadows,
  temperature, tint, vibrance; Wheels: lift/gamma/gain/offset; Curves incl. hue-vs-sat etc.; Looks/LUT .cube import;
  scopes: waveform, parade, vector, histogram; hold to compare; copy/paste grade), "Swap this clip in / Alternatives /
  Describe the b-roll to swap in…" (sends request to agent).
- Export dialog: Video File / GIF / Premiere Pro (XML) / Final Cut-DaVinci (FCPXML); resolution (to 4K by plan), fps,
  quality, codec, format; GIF colours + dithering; time range; watermark (Lite) vs clean (paid) with compare toggle and
  inline plan upsell; local export (WebCodecs) or cloud export with media upload + verification; post-export checks
  (has video/audio track, decodes a frame).
- Error/edge states: tab crashed ("Your project files are safe. Reload tab"), missing files → relink (file or
  folder), unsupported codecs, file size limit by plan, graphic failed to render ("Ask built-in agent to fix it").
- Developer overlays: renderer perf, memory/CPU, decoder/cache counters; Browser DevTools dock with LLM stream trace.

3.5 Settings (/app/settings; left nav, "Up to date · 1.5.0" at bottom)
- Library: location, Change…, Show in Finder, projects biggest first, temporary files.
- Agents: On this Mac (Claude Code, Codex: sign in / change account / sign out / enable toggle, Check again), Agent
  tools (Install = `<cli>` + skills into ~/.local/bin and agent skill dirs), Chat: "Show the agent's work".
- Account: signed in as, plan, sign out.
- Usage: usage credits, trial usage, generative credits, agent time, top-up balance, history, Get more.
- Shortcuts: presets (Default, Premiere Pro, DaVinci Resolve, Final Cut Pro, CapCut), rebind, search, reset.
- Privacy: share data to improve (PostHog/Sentry), crashes, built-in agent data.
- Developer: overlays, overlay position.

3.6 Global: Report an issue (text + screenshots + screen recording), update flow, billing dialogs (Choose a plan,
Update payment, Top up, Keep my plan), thanks/early-user notes.

---------------------------------------------------------------------------------------------------------------------
## 4. Main flows

A. First run: install DMG → sign in (Google via browser + code) → invite check → welcome survey → agent consent →
   Home.
B. Prompt-first: Home prompt + media/aspect → new project bundle → editor opens with chat running built-in agent → agent
   reads AGENTS.md / timeline.tsx header → asks via ask_user card if needed → places pending items → fills them one per
   save → verify + look → reports changes in items.
C. Recipe: pick recipe → recipe folder copied into bundle → inputs (subject/story/media) → agent follows RECIPE.md,
   only writes story + casting into fixed templates.
D. Manual: drag media → timeline; select → properties; left-rail panels add text/captions/voice/effects; every manual
   change also regenerates timeline.tsx so an agent sees it.
E. External agent: Settings › Agents › Install agent tools → user's Claude Code/Codex edits bundles via `<cli>`; the open
   app updates live; or remote MCP from ChatGPT/Claude with in-chat player.
F. Export: dialog → local WebCodecs render (or cloud pool) → checks → file; NLE timeline export for Premiere/Resolve.
G. Money: plan (Lite/Starter/Creator/Pro), usage credits (agent + compute time), generative credits (models), top-ups;
   exit code 4 / upsell cards when out.

---------------------------------------------------------------------------------------------------------------------
## 5. Weak spots observed (where a better product can win)

1. Agent cost and latency on intake. The deleted demo project's chat log showed one turn that ended waiting on an
   ask_user question after 44 min 22 s, using 1,737,400 input tokens (1,669,632 cached). The recipe's
   `inputs.json` was empty (subject, story, media all null), so the agent was started before the required inputs were
   collected. A structured intake form before the agent starts would avoid this.
2. Weight: 300 MB DMG; 277 MB of it is a bundled agent binary. ffmpeg 6.0 and ffprobe 4.4.1 are mismatched versions.
3. Memory: one full renderer per tab (the app itself warns about per-tab memory); needs sleeping tabs.
4. Pricing complexity: usage credits + generative credits + agent time + top-ups + export limits + watermark.
5. Speech-to-text is cloud-only (local transcription disabled under resource pressure), so offline editing is weak.
6. Gating friction: invite list, Google-only sign-in, internet required to set up.
7. Two parallel editing models (GUI props vs file) require merge/conflict logic everywhere; it is well done, but a new
   product should design the document model once with the merge engine as the core, not a bolt-on.
