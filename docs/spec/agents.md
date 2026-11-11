# Agents and templates

## Providers

| Provider | Runs where | Auth | How it is driven |
|---|---|---|---|
| Hosted (built-in) | Bundled Claude Code runtime via the Claude Agent SDK, on the user's Mac | the app's account; LLM calls go through the vendor's Messages-compatible proxy, metered per plan | SDK `query()` with `permissionMode: bypassPermissions`, `settingSources: []`, no user skills/plugins/agents, `strictMcpConfig`, persisted session, `systemPrompt: {preset: claude_code, append: <app rules + one skill>}`, model Opus-class with 1M context, effort default medium; long-lived turn, steerable mid-turn |
| Claude Code (BYO) | user's installed `claude` | user's Claude subscription | `claude -p --verbose --input-format stream-json --output-format stream-json --include-partial-messages --permission-mode bypassPermissions --append-system-prompt <rules> [--model] --effort <e> [--resume <session>]`; user messages written as stream-json lines |
| Codex (BYO) | user's installed `codex` | user's ChatGPT plan | `codex exec [resume <id>] --json --skip-git-repo-check -c sandbox_mode="danger-full-access" -c approval_policy="never" -c features.code_mode_host=true -c mcp_servers.<app_ui>.url/bearer_token_env_var/tool_timeout_sec [--model] -c model_reasoning_effort=<e> "<rules>\n\n<message>"` |
| Gemini-based | bundled, present in enums, not exposed in UI | — | — |

Effort levels: low, medium, high, xhigh, max (ultra for some). Model/effort picker lives in the chat composer.
Working directory = the project bundle. Env is sanitised: all `ANTHROPIC_*`, `CLAUDE_*`, `OPENAI_*`, `OPENROUTER_*`,
`AZURE_OPENAI_*`, `GEMINI_*` and app tokens are stripped before launch, then the provider's own are set.

A separate tiny one-shot call (no tools, read-only, low effort) names new projects/chats from the first message.

## App-rules prompt (paraphrased, appended to every provider)

- You are editing project X; its bundle is the cwd. Work through the bundled CLI (on PATH); `docs` lists topics.
- Follow the app's own skill over any other media skill.
- Change the timeline only via the projection file or `source apply`; never write project.json. Use `--eval` scripts
  for loops.
- Stay inside the bundle; user media is read-only; scratch work in `.scratch/`; refuse edits outside the project
  except a file the user asks for at a named location.
- Generation: start with `--no-wait`, place with `--start`, report "started" not "ready". A still that only seeds a
  clip is generated as part of that clip (one paid confirmation).
- Grade by measuring frames, not by eye; mattes come from the hosted matte job.
- Verify with `look` and `verify`; export only when the user asks for the file.
- Replies: a few plain sentences in terms of items and times; no narration of steps, no tool/file names except files
  the user needs (as Markdown links).
- Ask through the ask_user tool (questions written as text do not pause the turn); wait on one UI tool at a time.

## In-app MCP server (`app_ui`, always loaded)

- `ask_user` — 1–4 questions, each with header (≤12 chars), single-sentence question, multiSelect, 2–4 unique options
  (label 1–5 words + one-sentence trade-off), recommended first. Returns `Q:/A:` pairs, or "dismissed — stop and wait".
- `pick_voice` — 2–4 voice ids with reasons; the UI plays samples; always offers "Your own voice" (→ record + clone).
- `clone_voice` — opens record/clone card (name, optional sample paths); returns new voice id or "nothing charged".

The agent "parks" on these tools; the chat panel renders them as cards with Skip / Back / Next.

## Chat skills (toggleable guardrails)

Accessibility (caption all speech, readable text duration/size, flash limits, duck music under speech), Brand-safe
(no profanity/sensitive jokes, calm music, natural grade, sentence-case text, no real people/logos in generations),
Compliance (profanity bleep + subtitle censor, obscene gesture blur with tracking, branding blur that follows motion,
bumpers; flag uncertainty).

## Templates ("recipes")

Catalog entry: `id, slug, name, description, category, audience, isTrending, onWeb, usesGeneration, prompt,
bodyMarkdown, highlights[], inputs[], agent{provider,model,effort}, aspectRatio, otherAspectRatios[], length
(short|long), posterUrl, previewUrl, videoUrl, assets[{id,name,kind,url,sha256,bytes,width,height,durationMs,bindings}],
kit[{path,url,sha256,bytes}], samples[]`. Home layout blocks: trending rail + catalog rows.

Inputs: `text {id,label,placeholder,optional,maxLength}`, `media {id,accept[video|image|audio],min,max,prompt,hint}`,
`select`.

Kit files copied into `recipe/`: `RECIPE.md` (agent instructions), `structure.json` (length, narrator voice + speed,
scenes with start/end, narration word budgets, media slots), `inputs.json` (filled values), `templates/*.jsx` (all look
and motion as `<code>` templates with props), `shaders/*.glsl` + `shaders.json`, `guide.jpg` (labelled key frames),
optional `STORY.md`, `reference.jpg`, `tools/*.py`, `models/*.onnx`, `sfx-cues.json`, font licence files.
The agent writes only story + casting into fixed templates; it never authors graphics for a template.

Catalog at capture: 86 templates — Effect 25, Viral trends 14, Launch video 8, Product showcase 8, Clipping 6, Vlog 6,
UGC 5, Travel montage 5, Talking-head 4, Aftermovie 3, Explainer 2. Aspects: 9:16 ×41, 16:9 ×35, 4:3 ×6, 4:5 ×2,
1:1, 3:4. 71 short / 15 long; 8 use generation. Agent effort: high for 73.
