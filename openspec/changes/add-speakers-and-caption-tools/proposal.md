# Proposal: add-speakers-and-caption-tools

## Why

Interviews and podcasts need to know who speaks when: to cut to the speaker, to colour the waveform, and to label
captions. Captions also need more control than fixed 42-character chunks: words per caption, lines, gap closing, a fit
check against the real text layout, subtitle files for platform uploads, and other languages. Today there is no
speaker identity, caption segmentation has no settings, there is no SRT or WebVTT import or export, and there is no
translation. This change adds all of it, on this computer, with the agent as the only translation engine.

## What Changes

- Speaker identity: on-device speaker detection per media file, matched across all files of a project so the same
  person gets the same speaker everywhere. A project speaker list (name, colour) that people and agents can rename,
  merge and split. Speaker runs in the timeline transcript. Waveforms tinted by speaker.
- Caption building settings: maximum words, characters and lines per caption; a fit check that uses the same text
  layout as export; closing short gaps between captions; text case.
- Subtitle files: import SRT and WebVTT as caption items on a new caption track; export caption tracks as SRT or
  WebVTT, optionally with word timestamps in WebVTT.
- Caption style gallery previews rendered by the same renderer as export.
- Per-word caption styles: colour, scale and emphasis for single words, visible in `timeline.tsx`.
- Translation: the assistant (or an external agent) translates a caption track into a new caption track with the same
  timings, as one revision.
- Surfaces: CLI (`ove speakers`, `ove audio captions` flags from docs/spec/cli.md, `ove captions import|export|translate`),
  MCP tools (`identify_speakers`, `caption_from_transcript` options, `import_subtitles`, `export_subtitles`,
  `translate_captions`), the Audio panel Speakers section, and Captions panel controls.

Out of scope: overlapping-speech separation, local machine-translation models, behind-subject captions (add-captions
task 1.4), voice cloning and dubbing (extend-ai-generation-actions), word removal and dead air
(add-transcript-editing), profanity masking.

References: docs/spec/cli.md (`audio captions`), docs/spec/mcp-tools.md (`caption_from_transcript`,
`transcribe_media` formats `srt` and `vtt`), docs/spec/document-model.md (project.json, Items, Projection rules),
docs/spec/presets.md (captions).

## Capabilities

### New Capabilities
- `speaker-identity`: speaker detection, cross-file matching, the project speaker list, speaker runs in the
  transcript, and speaker-tinted waveforms.
- `caption-tools`: caption building settings, subtitle import and export, rendered gallery previews, per-word styles
  and caption translation.

### Modified Capabilities
- None. `captions` keeps its requirements; this change adds options and new caption sources.

## Impact

- `crates/ove-analyze`: speaker embedding and clustering, cached per media file.
- `crates/ove-transcribe`: segmentation options, subtitle parser and writer.
- `crates/ove-core` / `crates/ove-projection`: optional project speaker list, per-media speaker overrides and per-word
  caption style fields, with migration and projection round trip.
- `crates/ove-render`: gallery preview rendering, per-word style rendering.
- `crates/ove-cli`, `crates/ove-agent`, `apps/desktop` (Audio and Captions panels).
- Depends on: `add-transcript-editing` (timeline transcript, speech masks), `add-model-runtime` (speaker model download
  and inference), `add-audio-mixing-and-monitoring` (waveforms on clips, for speaker tint),
  `add-background-tasks-and-media-derivatives` (content identity for cache keys; background analysis).
