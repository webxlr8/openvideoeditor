# Proposal: improve-interchange-and-packaging

## Why

Timeline exchange today carries cuts and media only. Transform, crop, opacity, volume, keyframes and fades are reported
as "not carried" for every format, so a hand-off to a finishing editor loses the framing and the mix. Some editors read
FCPXML values differently from the published format, so one FCPXML file cannot be right for every target. OTIO, our
richest format, also drops what it could store. Projects that reference media outside the bundle cannot be moved to
another computer as one folder.

## What Changes

- Interchange model: clips carry transform (position, scale, rotation, flip), crop, opacity, volume in dB, keyframes for
  these properties, enabled state and source timecode.
- FCPXML: a target choice (DaVinci Resolve or Final Cut Pro) that changes how position, scale, rotation, crop, retimes,
  keyframes and audio-only or video-only use of a source are written; a version choice (1.10 to 1.14).
- FCP7 XML (Premiere Pro): motion (position, scale, rotation) and opacity filters, and volume levels.
- OTIO: every item keeps all of its fields under an `ove` metadata namespace, so export then import is lossless.
- Import reads back the same properties for every format.
- Fidelity report per target and version, shown before writing.
- Project packaging: copy a project and every media file it uses into one self-contained bundle, with deduplication by
  content, optional trim-to-used with handles, optional proxies, optional zip, a free-space check, and a report.
- Surfaces: `ove timeline export --target resolve|fcp --version 1.10`; `ove project package`; MCP `export_timeline` and
  `import_timeline` gain target and version; MCP `package_project`; export dialog target and version pickers; File →
  Package Project.

Out of scope: AAF, Premiere `.prproj`, carrying colour grades, effects, transitions beyond cross dissolves, or text
styling across editors; multicam and nested timelines in interchange (after `add-timelines-nesting-and-multicam`);
fonts bundling (listed only); collecting media for other users' projects over a network.

References: docs/cli/interchange.md, docs/spec/desktop-shell.md (document types, archive), docs/spec/document-model.md
(media registry, ownership, relink), docs/spec/mcp-tools.md (timeline export / import, `relink_media`).

## Capabilities

### New Capabilities
- `interchange-fidelity`: the extended interchange model, target- and version-aware FCPXML, motion and opacity in FCP7
  XML, lossless OTIO, import of the same properties, and per-target fidelity reports.
- `project-packaging`: self-contained project packages with content deduplication, trim-to-used, proxies, zip, space
  check and report.

### Modified Capabilities
- None.

## Impact

- `crates/ove-interchange` (`model.rs`, `fcpxml.rs`, `xmeml.rs`, `otio.rs`, `report.rs`, import paths).
- `crates/ove-media` (source timecode in probe; shared with `add-audio-intelligence`, which owns timecode probing for
  sync — whichever lands first adds it).
- New `crates/ove-store/src/package.rs`; package jobs run in the export queue.
- `crates/ove-cli` (`timeline export|import` flags, `project package`), MCP handlers, export dialog and File menu in
  `apps/desktop`.
- Depends on: `extend-export-formats-and-queue` (queue; transcodes for trim-to-used).
