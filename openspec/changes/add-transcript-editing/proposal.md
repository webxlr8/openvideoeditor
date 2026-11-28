# Proposal: add-transcript-editing

## Why

Talking-head, interview and podcast edits start from what people say. Today the editor transcribes speech for captions
only. A person or an agent cannot cut the video by deleting words, cannot remove dead air, and cannot search what was
said across the media. Agents have no transcript tool at all, although docs/spec/mcp-tools.md lists
`transcribe_media` and `search_transcript`. This change makes the transcript an editing surface for people and agents,
fully on this computer, on macOS and Windows.

## What Changes

- Timeline transcript: words from every clip on the timeline, mapped through trim, speed and direction into timeline
  time, with word ids anchored to the source so they stay valid after cuts. One shared implementation for the app, the
  CLI, MCP and the in-app assistant.
- Faster and more accurate recognition: GPU-accelerated recognition where the computer supports it (CPU fallback), and
  an optional "accurate" model downloaded on first use with consent.
- Remove words: delete selected words (by id, by range or by exact match) from the timeline in one revision. Cut edges
  keep part of the natural pause (tight, balanced or loose), snap to the quietest point near the word boundary, and get
  a short fade at each seam. Caption items that cover the cut words follow the cut.
- Remove dead air: find pauses with on-device voice activity detection, relative to the speaker's own level so music
  beds and loud ambience stay, then cut them with minimum-pause and padding settings, in one revision.
- Filler words: a verbatim recognition option and filler candidates (filler words from a per-language list, and voiced
  gaps with no recognised word). Fillers are suggested for review, never cut without confirmation.
- Transcript search: phrase search across all transcribed media with word-accurate times, each hit also listing where
  that source range is used on the timeline.
- Dry run: every cutting command can return the resulting `timeline.tsx` diff without applying it.
- Surfaces: CLI (`ove transcript`, `ove transcript search`, `ove edit remove-words`, `ove edit remove-silence`), MCP
  (`transcribe_media`, `get_transcript`, `search_transcript`, `remove_words`, `remove_silence`, `detect_silence` with a
  voice-activity mode), assistant tools with the same contracts, and a Transcript panel where struck-through words are
  cut with Delete.
- Takes over add-captions task 1.5 (assistant transcript tool and remove-silences action).

Out of scope: speaker identification and per-speaker colours, caption layout settings, subtitle file import and
export, and caption translation (all in add-speakers-and-caption-tools); cloud recognition services; overlapping-speech
handling; editing caption styles; behind-subject captions (add-captions task 1.4).

References: docs/spec/cli.md (`audio transcribe`, `audio silence`), docs/spec/mcp-tools.md (`transcribe_media`,
`search_transcript`, `detect_silence`), docs/spec/document-model.md (Items, Apply / merge, Verify).

## Capabilities

### New Capabilities
- `transcript-editing`: the timeline transcript with stable word ids, word removal, dead-air removal, filler
  candidates, transcript search, dry-run diffs, and the CLI, MCP, assistant and panel surfaces that expose them.

### Modified Capabilities
- None. `captions` keeps its requirements; caption items gain the behaviour of following cuts, specified here.

## Impact

- `crates/ove-transcribe`: new headless timeline-transcript, cut-planning and search modules; GPU recognition builds;
  verbatim option.
- `crates/ove-analyze`: speech activity masks and dead-air detection, cached in the bundle.
- `crates/ove-cli` (commands and MCP tools), `crates/ove-agent` (assistant tools), `apps/desktop` (Transcript panel,
  dead-air preview overlay).
- Depends on: `add-edit-planners-and-links` (ripple-delete planner and per-timeline frame grid), `add-model-runtime`
  (model download and inference runtime for voice activity detection), `add-background-tasks-and-media-derivatives`
  (content identity for cache keys; background transcription), `add-audio-mixing-and-monitoring` (waveforms on clips,
  used by the dead-air preview overlay). `unify-agent-tool-layer` is preferred but not required: tools are added to
  whichever tool registry exists when this change starts.
- Closes add-captions task 1.5.
