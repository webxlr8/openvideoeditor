# Proposal: add-audio-intelligence

## Why

Productions record sound on separate devices and film with several cameras. Lining these up by hand is slow and is
the first job in almost every interview, podcast or event edit. Music videos and social edits are cut on the beat, but
the editor cannot find beats. Location audio is noisy, and today the only clean-up path is a paid cloud provider.
These three jobs (sync, beats and denoise) can run locally on both macOS and Windows. Each one should give agents a
precise, reviewable result that lands as one revision.

## What Changes

- Source timecode: probe reads embedded timecode from QuickTime timecode tracks and from Broadcast WAV and iXML
  metadata, and keeps it with the media entry.
- Audio sync: align chosen items to a reference item by timecode first, then by audio. Audio alignment uses a
  cross-correlation search over all offsets, a fine refinement to under 1 ms, and a drift measurement across the
  recording. Drift is corrected by retiming the audio recorder, not the camera. Placement can be finer than one frame.
  A dry run returns the plan without editing. Applying the plan is one revision.
- Beats: detect beats, downbeats, tempo and bars in music with a local beat-tracking model. Results are cached
  analysis, not document data. Beat times are reported both in source seconds and in timeline seconds for an item,
  taking trim, speed and reverse into account. Beats become snap targets on the timeline.
- Denoise: a per-item `denoise` amount (0–100) on video and audio items. A local speech enhancement model renders a
  cleaned copy of the sound once, in the background. The mixer blends clean and original by the amount. The blend is
  aligned to the sample. Export renders any missing cleaned copy and refuses to ship un-cleaned sound unless the person
  allows it.
- CLI, MCP and UI for each: `ove audio sync`, `ove audio beats`, `ove audio denoise`; MCP `sync_items`,
  `detect_beats`, `denoise_audio`; a timeline "Synchronise selection" command, a "Snap to beats" toggle and an
  Inspector denoise slider.

Out of scope: multicam groups and angle switching (add-timelines-nesting-and-multicam uses this sync); Sony
`rtmd` timecode; voice isolation and stem separation; cloud clean-up providers (stay in add-ai-generation); per-speaker
loudness matching; the snap engine itself (add-edit-planners-and-links); the model runtime and downloader
(add-model-runtime).

References: docs/spec/document-model.md (Items: sound fields; Projection rules), docs/spec/cli.md (`audio process`,
`audio levels`, `media add`), docs/spec/mcp-tools.md (audio and media tools), docs/spec/presets.md (snapping
shortcut).

## Capabilities

### New Capabilities
- `audio-intelligence`: source timecode, audio sync with drift correction, beat and downbeat detection with beat snap
  targets, and local denoise with a blend amount.

### Modified Capabilities
- None. The `denoise` field is an added sound field; existing documents stay valid.

## Impact

- Crates: `ove-media` (timecode probe, Broadcast WAV and iXML reader), `ove-analyze` (sync, beats), `ove-render`
  (denoise render and blend in the mixer, export rule), `ove-core` and `ove-projection` (`denoise` field and typings),
  `ove-cli` (commands and MCP tools), desktop app (sync command, beat snap toggle and beat marks, denoise slider and
  progress chip).
- Depends on: `add-model-runtime` (runtime and model downloads with consent), `add-background-tasks-and-media-derivatives`
  (scheduler for analysis and denoise renders), `add-audio-mixing-and-monitoring` (analysis cache and onset envelope,
  second mixer source), `add-edit-planners-and-links` (snap engine that consumes beat targets).
- Used by: `add-timelines-nesting-and-multicam` (sync places multicam members).
- New dependencies, each checked by cargo-deny: an FFT crate (MIT/Apache), the speech enhancement library and its
  inference crate (MIT/Apache). Model weights are checked separately (design.md Open Questions).
