# Proposal: add-audio-mixing-and-monitoring

## Why

Sound is half of every edit, but today the editor only has a static volume per clip. People cannot see waveforms on
the timeline, cannot hear audio while they scrub, cannot see levels while they play, and cannot shape volume over
time. The Duck button sets music to a flat level, and export has no loudness control. Agents have the same gaps: they
cannot duck music under dialogue or deliver a file at a platform loudness target. This change gives the mixer the
basic tools that every editor has, and makes each one available to people and agents through the same document
pipeline.

## What Changes

- Volume keyframes: `volume` becomes a keyframe property. The mixer evaluates it in decibels, block by block, and
  applies it in preview and export alike. Volume keyframes print in `timeline.tsx` like every other keyframe track.
- Import analysis cache: each media file is decoded once in the background at import. The pass writes a stereo
  min/max peak pyramid and a loudness summary to the bundle cache. Later analysis reuses this data and does not decode
  the file again.
- Clip waveforms on the timeline, drawn from the peak pyramid. They follow trim, speed and reverse, and they scale
  with the clip's gain.
- A gain line (rubber band) on each clip: drag the line to change the static volume, add and drag points to make
  volume keyframes. One gesture is one revision.
- Stereo meters with peak hold, a clip indicator, true peak and momentary and short-term loudness, aligned with what
  the person hears.
- Scrub audio: short windowed grains of the real mix play while the playhead is dragged.
- Real ducking: music is lowered under dialogue. Speech comes from transcript word timings or from a level gate on
  the dialogue items. The result is volume keyframes on the music items, so it stays editable. One revision. CLI,
  MCP and a panel control.
- Loudness on export: a true-peak look-ahead limiter replaces the sample-peak limiter, and export presets normalise
  the mix to a loudness target (off, −14, −16, −23, −24 LUFS). The export result reports loudness before and after.
- Audio-only export (AAC in `.m4a`, WAV 24-bit, FLAC) and "extract audio" from a media file into the library.

Out of scope: machine-learning models, audio sync, beat detection and denoise (add-audio-intelligence); keyframe
lanes for other properties and the shared lanes panel (add-markers-keyframe-lanes-and-versions); video codecs and the
export queue (extend-export-formats-and-queue); surround output; plug-in hosting (VST, AU); pitch-preserving shuttle
audio above 1×; per-speaker loudness matching.

References: docs/spec/document-model.md (Items: sound fields and keyframeTracks; Projection rules),
docs/spec/cli.md (`audio waveform`, `audio levels`, `audio loudness`, `export`), docs/spec/mcp-tools.md
(`measure_loudness`, export tools), docs/spec/presets.md (shortcuts for scrub and playback).

## Capabilities

### New Capabilities
- `audio-mixing`: volume keyframes, the import analysis cache, clip waveforms and the gain line, meters, scrub audio,
  dialogue-driven ducking, loudness-normalised export with a true-peak limiter, audio-only export and extract audio.

### Modified Capabilities
- None. The keyframe property list grows by one value; existing documents stay valid without migration.

## Impact

- Crates: `ove-core` (keyframe property list), `ove-projection` (printing and typings for volume keyframes),
  `ove-render` (mixer, limiter, loudness core moved from `ove-analyze`, audio-only export), `ove-analyze` (peak
  pyramid, re-exported loudness), `ove-playback` (scrub grains, meter taps), `ove-cli` (`audio duck`,
  `media extract-audio`, export flags), desktop app (timeline waveforms, gain line, meters, duck popover, export
  dialog).
- Depends on: `add-background-tasks-and-media-derivatives` (scheduler and `contentId` for the analysis pass; until it
  lands, the pass runs on the existing import worker), `fix-render-and-edit-correctness` (the Duck and Normalize
  button labels), `extend-export-formats-and-queue` (shared export settings model; this change adds the audio kind).
- Used by: `add-audio-intelligence` (reuses the analysis cache and onset envelope), `add-transcript-editing` (quiet
  edge snapping reads the peak cache), `add-markers-keyframe-lanes-and-versions` (lanes UI edits volume keyframes).
