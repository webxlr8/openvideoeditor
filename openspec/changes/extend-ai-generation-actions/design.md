# Design: extend-ai-generation-actions

## Context

See proposal.md (Why). Today `ove-generate` has a hard-coded catalog (`catalog.rs`, 10 models, flat prices, one
capability flag), a request with one image path sent as a data URI, a background worker, jobs saved in the bundle,
`pending` placeholders that fill in place, and a confirmation before spending. Keys are bring-your-own in the OS
credential store, and `key_allowed` restricts which domains a key may be sent to. Requirements: specs/ai-clip-actions.

## Goals / Non-Goals

**Goals:**
- New models and new provider parameters without an app release, with no backend and no account.
- AI actions are non-destructive by default and every result is a revision that can be undone.
- Agents can use every action, but spend only inside a budget the person sets.

**Non-Goals:**
- A hosted catalog service, a proxy, credits or plans.
- Running generative models on the computer.
- Choosing the "best" model automatically beyond the catalog's defaults.

## Decisions

### D1. Catalog: embedded, cached, signed remote
Load order: the catalog embedded in the binary (today's models, migrated to the new schema), then the cached copy in
the app data directory, then the remote copy. The remote copy is a static `catalog.json` plus a detached ed25519
signature; the public key is compiled into the app. The app fetches it at most once a day with an anonymous GET that
sends no key, no user id and no project data. A signature failure, a schema version above the app's, or a parse error
keeps the previous catalog and logs a warning. Settings → Generation can turn remote refresh off or point it to a local
file or another URL (studios), which must also be signed by a key the person adds.
Alternatives: catalog in the binary only (a release for every model change); unsigned remote JSON (a compromised host
could redirect requests); a live backend subscription (needs accounts and a service, out of scope).

### D2. Catalog entry schema
Each entry: `id`, `provider`, `kind` (video, image, audio, upscale), `modes` (each with an endpoint and a request
template), `output` (a JSON pointer to the result URL), `caps` (durations, resolutions, aspects, first/last frame,
max reference images / videos / audios, reference tag noun, whether frames and references exclude each other, source
video limits and required encoding, audio toggle, draft `{resolution, enhance: rerun | upscale:<id>}`), `actions` (tags:
`upscale`, `reframe`, `lipsync`, `edit`, `v2sfx`, `v2music`, `dub`, `cleanup`, `voicechange`, `voiceclone`,
`transition`), `price` (rules per second by resolution, per 1,000 characters, flat, audio-off discount, draft rate),
`settings` (typed form fields), `minAppVersion`, `priceAsOf`. A request template maps our request fields to provider JSON
keys with simple placeholders only; it cannot run code. Endpoints must resolve to a domain already on the key's allow
list, otherwise the entry is rejected at load.
Alternatives: provider adapters in Rust for each model (a release per model); free-form templates with expressions
(hard to validate, risk of key exfiltration).

### D3. References and uploads
`Request.refs: [{role: first | last | image | video | audio | source, media | path, range?}]` replaces `Request.image`
(kept as an alias). The prompt keeps `@Image1`-style tags in the order of the refs of that kind; the catalog's reference
tag format rewrites them per provider. Validation fails when a tag has no ref or a ref exceeds the model's limits.
Refs are uploaded with the person's provider key to the provider's file storage (fal storage first), in parallel, and
cached in `cache/uploads.json` by content hash with the provider's expiry. The generated media records its lineage in
`data.generated` (model, mode, prompt, refs by media id and range, seed, settings), so rerun and agents can rebuild the
request exactly. The UI states that references are uploaded to the provider.

### D4. Preprocessing through the shared renderer
`prep.rs` describes the work as data: media range, timeline span, maximum short side, target encoding (H.264/AAC MP4,
M4A or WAV, PNG or JPEG still). The caller runs it with `ove-render` range export from `extend-export-formats-and-queue`,
so a timeline-span reference matches what the person sees, including effects and captions. "Use the trimmed range only"
is on by default. Temporary files go to `cache/prep/<job-id>/` and are deleted when the job ends.

### D5. Spend control
Every paid call has an estimate from the catalog price rules, shown in USD with `priceAsOf`. A local ledger
(`cache/spend.json` per project) records estimates and outcomes (done, failed, cancelled). Settings hold a budget per
project and a budget per agent session. A call that would exceed a budget needs the person's confirmation, even when
`unify-agent-tool-layer`'s quote token is valid. The ledger is labelled "estimates, not the provider's bill" and links to
the provider's usage page.

### D6. AI actions and takes
`actions.rs` defines each action: accepted media types, catalog tag, preprocessing, and placement:
- Visual results (upscale, reframe, lip sync, prompt edit, rerun, draft-to-enhance): a `pending` placeholder on the lane
  directly above the source clip, over the same span — a take. Takes are found through lineage
  (`data.generated.sourceItemId`); the document schema does not change. "Replace when done" instead applies one
  `UpdateItem` that swaps the clip's media id and keeps its trim, speed, transform, effects and volume.
- Audio results (video-to-SFX, video-to-music, dubbing, voice cleanup, voice changer): a placeholder on an audio lane at
  the same span.
- Voice clone creates a provider voice and needs a consent checkbox that states the person has the right to clone the
  voice; it creates no media.
Draft-to-enhance follows the catalog `draft.enhance` rule: rerun at a higher resolution with the same seed, or run the
named upscaler. Every placement and replacement is one revision.
Alternatives: replace by default (destructive, hard to compare); a new "take" item type (schema change across every
consumer for little gain).

### D7. AI transition
Input is a gap on a video track (at most 15 s) or a cut point with handles. The app grabs the composited frame just
before and just after with the shared renderer, picks a model tagged `transition` with first + last frame mode, and
places a placeholder in the gap. On finish: `fit: speed` changes the item's speed so it fills the gap exactly; `fit:
trim` requests the smallest supported duration that covers the gap and trims it, so motion is not distorted. At a cut,
the transition replaces the handle frames symmetrically. If the gap was filled while the job ran, the result lands as a
take above. Prompts for transitions and reframing are our own text, stored in the catalog so they can be tuned and
translated.

### D8. Surfaces
- CLI: `ove generate action <kind> <project> --item <id> [--model <id>] [--replace] [--range-only] [--prompt <text>]
  [--audio <media>]`, `ove generate transition <project> --track <id> --at <seconds> [--fit speed|trim]`, `ove generate
  models [--refresh] [--json]`.
- MCP: `ai_clip_action`, `generate_transition`, `list_models` returning catalog capabilities; all paid calls use the
  quote and confirm contract of `unify-agent-tool-layer` plus the budget rule in D5.
- UI: the Generate panel shows only modes and settings the selected model supports; a mention picker for references;
  a timeline context submenu "AI" and an Inspector "AI" section list available actions with the reason when one is not
  available; take lanes can be shown or hidden.

## Risks / Trade-offs

- [Provider endpoints and parameters change often] → endpoints live in the catalog, per-action contract tests run
  against local stand-in servers, and a weekly live smoke test uses a maintainer key outside CI. Detection: provider 4xx
  bodies are shown in the job error and counted in the smoke test report.
- [A malicious catalog redirects keys] → signature check, domain allow list per key, templates without code. Detection:
  unit tests for tampered, wrong-key and off-list-domain catalogs.
- [Estimates differ from bills] → label as estimates, show `priceAsOf`, link to the provider dashboard. Detection: the
  smoke test compares estimates with provider-reported cost where the API returns it.
- [Uploads stay on provider storage] → disclosure in the UI; uploads are not reused after expiry. Detection: cache
  expiry test.
- [Voice cloning misuse] → explicit consent checkbox per voice, recorded in the ledger; no clone without it.
- [Takes clutter the timeline] → take lanes can be hidden, and "keep this take" removes the others in one revision.

## Migration Plan

The embedded catalog is today's model list in the new schema; existing jobs and `--image` flags keep working through the
alias. No document migration.

## Open Questions

- Wave order: this change proceeds in the master-plan order (wave 3) unless the owner reprioritises.
- Remote catalog host and signing key owner are not decided; until then only the embedded catalog and a person-supplied
  signed file are used.
- Provider endpoints marked (inferred) in planning — Topaz/SeedVR upscale, reframe, sync lip sync, Wan-VACE / Luma
  modify, MMAudio / Mirelo sound effects, video-to-music, fal storage upload flow — must be verified against current
  provider documentation (task 2.1) before their catalog entries ship.
