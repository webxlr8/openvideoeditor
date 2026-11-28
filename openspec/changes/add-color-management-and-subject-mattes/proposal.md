# Proposal: add-color-management-and-subject-mattes

## Why

The pipeline is display-referred and 8-bit. HDR sources are tone-mapped to SDR at decode, log footage from modern
cameras and phones has no decode, and grading happens on display values. There is no way to export HDR. The schema
already has `region: subject|background` for effects and a `behind-subject` caption mode, but no matte is produced, so
both fall back to the whole frame with a label. Production users need correct colour from log and HDR sources, and
social creators expect captions and text that pass behind the person.

## What Changes

- Project colour settings: a working space (`rec709` default, `linear-rec709`, `acescct`, `dwg-intermediate`) and an
  output space (`rec709`, `rec2100-pq`, `rec2100-hlg`) with peak brightness for HDR.
- Per-media input colour space: detected from the file's tags, with a manual override. A `color-transform` item effect
  converts between named spaces with an optional tone map.
- Log decodes from published formulas: Apple Log, S-Log3, LogC3, LogC4, V-Log, Log3G10, plus sRGB, BT.1886, PQ and HLG
  transfer curves and the matching primaries.
- Pipeline: input transform, grade in the working encoding, composite in linear light, output transform with tone
  mapping, encode with correct colour tags. LUT entries gain a `space` field (default `rec709`).
- HDR output: Rec.2100 PQ and HLG with mastering display and content light metadata. HDR preview where the operating
  system and display allow it, otherwise a tone-mapped SDR preview of the same timeline.
- Subject mattes: an on-demand background job that computes a per-media person matte, smooths it over time and caches
  it in the bundle. The renderer refines it to full resolution along image edges.
- Mattes drive `region: subject|background` on effects, captions `behind-subject`, footage-filled text behind the
  subject, and `--region subject` in `ove color inspect`. `ove media matte <id>` starts the job; `ove look --view matte`
  shows it.
- Models and the inference runtime download on first use, with explicit consent for downloads above 50 MB.

Supersedes add-effects-and-color task 1.4 (subject and background regions). Closes the `behind-subject` fallback left
in add-captions.

Out of scope: OpenColorIO configs; Dolby Vision metadata; camera RAW decoding; click-to-select object segmentation;
mattes for subjects other than people; encoder selection for HEVC Main10 and ProRes (extend-export-formats-and-queue
supplies the encoders; this change supplies frames, tags and metadata).

References: docs/spec/project-schema-v1.md (`region`, caption `behind-subject`, `colorGrade`, LUT entries),
docs/spec/presets.md (LUTs, effects), docs/spec/timeline.d.ts, docs/spec/cli.md (`look`, `media`, `export`).

## Capabilities

### New Capabilities
- `color-management`: working and output colour spaces, input detection and override, the colour transform effect, log
  decodes, linear-light compositing, HDR output and preview.
- `subject-mattes`: on-demand person mattes, their cache, and every feature they drive (effect regions, captions and
  text behind the subject, region-limited colour readout).

### Modified Capabilities
- None. Defaults keep today's output: a project with no colour settings renders as it does now.

## Impact

- New `crates/ove-render/src/colorspace.rs`. Changes in `crates/ove-media/src/convert.rs` (no forced SDR at decode),
  `crates/ove-render/src/color.rs` (lattice in the working encoding), `crates/ove-gpu` (rgba16f end to end, HDR
  presentation).
- New `crates/ove-analyze/src/matte.rs` and an inference worker process.
- `crates/ove-core/src/schema.rs`: `project.color`, media `colorSpace`, effect `color-transform`, LUT `space`.
- CLI and MCP: `ove media matte`, `look --view matte`, `color inspect --region`, colour fields in `project` commands.
- Depends on add-gpu-compositor-and-scopes (float GPU pipeline, RGBA effect contract), add-model-runtime (model
  downloader and inference runtime), add-background-tasks-and-media-derivatives (job scheduling and `contentId` cache
  keys), extend-export-formats-and-queue (10-bit encoders). Uses add-viewer-interaction-and-text-fills (footage fill).
- New dependencies: none for colour (own implementation). Mattes use the shared inference runtime from
  add-model-runtime and a model with Apache-2.0 or MIT weights.
