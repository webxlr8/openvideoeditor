# Design: improve-interchange-and-packaging

## Context

See proposal.md (Why). `ove-interchange` exports and imports FCPXML 1.11, FCP7 XML, CMX3600 EDL and OTIO through a
small model of tracks and clips (media, timing, speed). `report.rs` lists what will not carry over before writing; it
lists transform, crop, opacity, keyframes, volume and fades as not carried in every format. Import adds new tracks as one
revision. Bundles already mark media as `ownedByBundle`, and `relink_media` exists as an op. Requirements:
specs/interchange-fidelity, specs/project-packaging.

## Goals / Non-Goals

**Goals:**
- A finishing editor opens our export with the framing, opacity and audio levels the person set.
- OTIO round trip without loss, so OTIO is a safe archive format.
- A project folder that opens on another computer with no offline media.

**Non-Goals:**
- Pixel-exact colour or effect matching across editors.
- Writing editor-native project files.

## Decisions

### D1. Extended interchange model
`model::Clip` gains `transform {x, y, scale, rotation, flip_x}`, `crop {l, t, r, b}`, `opacity`, `volume_db`, `fades`,
`keyframes: [{property, points}]`, `enabled`, and `source_timecode` (rational frames and rate, from the media probe).
All values use our document units; each writer converts. The model stays format-neutral.
Alternative: convert per format directly from the document (duplicated logic, harder to test).

### D2. FCPXML target and version
`fcpxml.rs` takes `Target { Resolve, FinalCutPro }` and `Version { 1.10 ..= 1.14 }`; default target Resolve, default
version 1.10 (the widest accepted). The writer keeps one code path and switches value encodings per target, based on
our own round-trip measurements:
- Position: Final Cut uses the published units; Resolve reads position relative to frame height and to the conform-fit
  scale, so the writer converts and records the conversion in a comment.
- Scale: relative to the conform fit for Resolve.
- Rotation sign and crop representation per target.
- Audio-only or video-only use of a source with both: written as a compound reference for the target that ignores
  per-clip enable flags.
- Retimes: a time map over the whole media for the target that needs it.
- Keyframe times offset by the clip start where the target expects clip-relative times.
- Volume in dB.
The exact factors are fixed by round-trip tests in Resolve 19/20 and Final Cut Pro 11 (task 2.3) and recorded in
`docs/cli/interchange.md`.
Alternatives: one encoding following the published format only (wrong framing in Resolve); a separate writer per target
(duplicated code for the 90 % that is shared).

### D3. FCP7 XML motion and opacity
`xmeml.rs` writes the Basic Motion filter (centre, scale, rotation), Crop filter, Opacity filter and Audio Levels
keyframes. Import reads the same filters.

### D4. Lossless OTIO
Each OTIO clip carries the full document item (all fields, including unknown ones) under `metadata.ove`, plus a format
version. Import prefers `metadata.ove` when present and its version is supported, and falls back to the OTIO-native
fields otherwise. Other tools keep unknown metadata, so a round trip through them keeps our data unless they drop it,
which the import report then shows.

### D5. Import of extended properties
Each reader maps the target-specific encodings back through the same conversion tables as the writer, so export then
import returns the same values within stated tolerances (position ±0.5 px, scale ±0.1 %, rotation ±0.01°, volume ±0.1
dB). Import stays one revision that adds tracks.

### D6. Fidelity report per target
`report.rs` lists, for the chosen format, target and version, every property used in the timeline that will not carry,
with counts and example item names. The export dialog shows it before writing; the CLI prints it, and `--json`
returns it; the report is part of the MCP result.

### D7. Packaging pipeline
`ove-store::package` runs as a job in the export queue:
1. Check free space on the destination against the estimated size (media bytes plus 10 %); refuse when short.
2. Copy the bundle, including the revision log, to `<dest>.partial`.
3. Copy each media file once, deduplicated by content id (size plus hashes of three 1 MiB samples, then a full hash on
   a match), into `media/`, with unique names.
4. In the copy, apply one `RelinkMedia` batch that points every media entry at its packaged file and marks it
   `ownedByBundle`. Nothing writes `project.json` directly.
5. Optional `--trim-to-used --handles <s>`: transcode only the used ranges plus handles (ProRes 422 HQ or H.264 via the
   export pipeline) and adjust item in-points in the same batch.
6. Optional `--include-proxies`, optional `--zip`.
7. Rename `<dest>.partial` to `<dest>` and write `package-report.json` (files collected, missing, deduplicated, bytes,
   fonts used but not included).
Missing media stays referenced and is listed in the report; packaging does not fail because of it unless `--strict`.
Alternative: copy files and rewrite paths in `project.json` (breaks the document pipeline rule and the revision log).

### D8. Surfaces
- CLI: `ove timeline export <project> <out> --format fcpxml --target resolve --version 1.10`; `ove timeline import` reads
  extended properties automatically; `ove project package <project> <dest> [--trim-to-used --handles 2] [--include-proxies]
  [--zip] [--strict]`.
- MCP: `export_timeline` and `import_timeline` gain `target` and `version`; `package_project` returns a job id.
- UI: target and version pickers in the timeline-export dialog with the fidelity report; File → Package Project with
  options, progress in the Exports panel and the report at the end.

## Risks / Trade-offs

- [Editors change their FCPXML reading between releases] → target and version are explicit, conversion tables are
  versioned, and a manual round-trip checklist per editor release lives in `docs/cli/interchange.md`. Detection: the
  integration task compares re-rendered frames with SSIM.
- [OTIO tools drop custom metadata] → import report shows when `metadata.ove` is missing and falls back to native
  fields. Detection: round-trip test through the OTIO reference library's adapters.
- [Packaging a large project fills the disk] → free-space check before copying and `.partial` staging. Detection: test
  with a destination too small.
- [Content-id collision on edited files of identical size] → confirm with a full hash before deduplicating. Detection:
  unit test with two files that differ outside the sampled regions.
- [Trim-to-used re-encodes lose quality] → ProRes 422 HQ default, handles configurable, originals untouched.

## Migration Plan

No document migration. Existing FCPXML exports default to target Resolve, version 1.10; `--version 1.11` keeps today's
output version.

## Open Questions

- Wave order: this change proceeds in the master-plan order (wave 4) unless the owner reprioritises.
- Default FCPXML target: Resolve (proposed, the most common cross-platform finishing tool) or Final Cut Pro.
- Trim-to-used intermediate on Windows: DNxHR HQ or ProRes through the FFmpeg encoder; follows the decision in
  `extend-export-formats-and-queue`.
