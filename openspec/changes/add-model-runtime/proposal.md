# Proposal: add-model-runtime

## Why

Several planned features run small machine-learning models on this computer: voice activity detection, speaker
embeddings, beat detection, visual search and subject mattes. Today only whisper has a model downloader, and it is
specific to whisper (`crates/ove-transcribe/src/models.rs`). If each feature adds its own downloader and its own
inference runtime, the installer grows, idle memory grows, and the two platforms drift apart. One shared runtime keeps
the installer within 60 MB, keeps idle RAM within 250 MB, and gives every model the same consent, checks and storage
controls on macOS and Windows.

## What Changes

- A shared model manager: model manifests with one or more files, each pinned by SHA-256 and size, fetched from
  commit-pinned URLs, resumed after interruption, checked for free disk space first, written as partial files and
  renamed only after the hash matches.
- Consent: a model larger than 50 MB downloads only after the person confirms in the app. An agent or CLI call never
  starts such a download; it returns a "model not installed" result with the size and a hint.
- Inference runtime: a measured spike compares a dynamically loaded ONNX runtime with a pure-Rust runtime on two
  reference models (voice activity and beat detection). The winner becomes the default. The native runtime library,
  if chosen, downloads on first use like a model, so the installer does not grow.
- Out-of-process inference worker: heavy models run in a child process that exits after it is idle, so the editor
  process keeps its idle memory budget. Light models (under 10 MB) may run in-process.
- Whisper models move onto the shared manager without a second download for people who already have them.
- Settings → Storage lists installed models with size, purpose and a Remove button; `ove models list|install|remove`
  and an MCP `list_models_local` tool report the same state.

Out of scope: any specific feature model (each feature change pins its own model in a manifest), the background task
scheduler (`add-background-tasks-and-media-derivatives`), GPU execution providers beyond what the spike measures,
model training or conversion pipelines beyond the parity check, cloud inference.

References: docs/03-architecture-plan.md (quality targets, dependency licence rule), docs/spec/cli.md (command
conventions, exit codes), docs/spec/mcp-tools.md (tool conventions).

## Capabilities

### New Capabilities
- `model-runtime`: installing, checking, listing and removing local models; consent for large downloads; running
  inference on both platforms within the memory budget.

### Modified Capabilities
- None. The whisper download path in `add-captions` keeps its behaviour; only its storage moves.

## Impact

- New crate `ove-models` (manifest, downloader, store, inference adapter, worker protocol) and a worker entry point in
  the `ove` binary (`ove model-worker`).
- `ove-transcribe` uses `ove-models` for whisper files.
- Settings gains a Storage section (shared with `add-background-tasks-and-media-derivatives`, which adds cache rows).
- Used by: `add-transcript-editing`, `add-audio-intelligence`, `add-semantic-media-search`,
  `add-speakers-and-caption-tools`, `add-color-management-and-subject-mattes`.
