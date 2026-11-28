# Proposal: add-semantic-media-search

## Why

Editors and agents spend most of their time finding the right moment: "the shot of the harbour at dusk", "where she
says the price". Today the media panel filters by file name only, and agents can only grab frames or scan contact
sheets. Searching footage by what it shows and what is said, on this computer, turns hours of logging into a query, and
lets an agent place the result on the timeline directly.

## What Changes

- Visual search: a local image–text embedding model indexes sampled frames of each video and each image. A text query
  (or "more like this frame") returns ranked shots with a best frame and a suggested in/out window.
- Shot-aware sampling: frame-accurate cut detection in one decode pass, sampling per shot, choosing the sharpest frame
  near each sample point, skipping black or flat frames. The cut list is cached and reused by scene detection.
- An index per media file stored in the project bundle, keyed by content identity, readable without the app running.
- Spoken search: ranked phrase search over the transcripts the app already makes, with word-accurate hit ranges.
- Media panel search with scopes (Moments, Spoken, Timelines, Files), result cards that drag onto the timeline with
  in/out already set, and an index status indicator.
- Organisation as document data: folders, tags, saved subclips (named source ranges) and smart folders (saved
  queries), all changed through the normal apply pipeline so they merge and undo like any edit.
- Agent tools: `search_media` (results include a ready-to-paste clip fragment for `timeline.tsx`), `media_transcript`,
  `inspect_media` with an overview sheet, and `organize_media`; CLI equivalents under `ove media`.

Out of scope: semantic (meaning-based) transcript search, approximate-nearest-neighbour or vector-database indexes, a
global index shared across projects, song identification, documents as assets, unused-media clean-up, model
installation itself (`add-model-runtime`), the scheduler (`add-background-tasks-and-media-derivatives`).

References: docs/spec/mcp-tools.md (`list_media`, `search_transcript`, `detect_scenes`, `grab_frame`, `filmstrip`),
docs/spec/cli.md (`media` group), docs/spec/document-model.md (media items, projection header media table, Projection
rules).

## Capabilities

### New Capabilities
- `media-search`: indexing footage, visual and spoken search, search results as clip ranges, media organisation as
  document data, and the agent and CLI tools that expose them.

### Modified Capabilities
- None as requirements. Scene detection returns the same results and may answer from the cached cut list.

## Impact

- New crate `ove-vision` (sampler, embedding calls through `ove-models`, index format, ranking); transcript search in
  `ove-transcribe`; media panel UI rebuilt as `ui/src/media/`; new rows in the MCP tool table and new `ove media`
  commands; schema additions for folder, tag, subclip and smart-folder fields on media and project data.
- Depends on `add-model-runtime` (model install, inference worker, consent) and
  `add-background-tasks-and-media-derivatives` (scheduler, content identity, skim tiles for result thumbnails).
- Agent tool registration goes through the registry from `unify-agent-tool-layer` if it has landed; otherwise through
  the current MCP table.
