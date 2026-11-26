# Design: add-speakers-and-caption-tools

## Context

See proposal.md (Why). Builds on `captions` (21 styles, caption items with word timings), `add-transcript-editing`
(timeline transcript, speech masks) and `add-model-runtime`. Today segmentation uses fixed limits (42 characters,
3.5 s, 0.6 s pause) in `ove-transcribe/src/segment.rs`, and the gallery previews are drawn in CSS, which can differ from
the export. Requirements: specs/speaker-identity, specs/caption-tools.

## Goals / Non-Goals

**Goals:**
- The same person gets the same speaker in every file of a project, fully offline.
- Speaker names and colours are document data, so renames are revisions and agents can change them.
- Caption layout limits that match what export draws.
- Subtitle files in both directions.

**Non-Goals:**
- Separating overlapping speakers; machine translation without an agent; profanity masking.

## Decisions

### D1. Speaker detection: embeddings and clustering on this computer
Over speech cells from the voice-activity mask, compute a speaker embedding for 1.5 s windows with a 0.75 s hop.
Cluster each file's windows by agglomerative clustering with a cosine threshold. Each word gets the majority label of
the windows it overlaps. Model candidates: a CAM++-class or ResNet34-class speaker model in ONNX form, about 26–28 MB,
run through the shared runtime. The weight licence is checked before pinning (Open Questions).
Alternatives: speaker labels from a cloud recogniser (not offline, costs money); pitch heuristics (fail for similar
voices).

### D2. Cross-file matching: assignment, not first-come
Each file's clusters are matched to the project registry centroids with an optimal one-to-one assignment (Hungarian
method) above a similarity threshold. Unmatched clusters become new speakers. Centroids are updated as a weighted mean.
Alternative: greedy matching in processing order (results depend on the order files were analysed).

### D3. Where speaker data lives
- Analysis results (embeddings, clusters, centroids) are cache data in the bundle, keyed by content identity.
- The project speaker list (`id`, `name`, `color`) is document data, changed with the existing project update op.
- Per-media corrections (merge two speakers, split one) are a per-media `speakerMap`, changed with the media update op.
- The `timeline.tsx` header prints the speaker list, so an agent renames "S2" to "Host" through apply.
Alternatives: store everything in the cache (renames would not be revisions and would not merge); new op kinds (more
schema surface for no gain).

### D4. Caption building
Segmentation gains `maxWords`, `maxChars`, `maxLines`, `maxGap` (0–2 s, default 0.5 s), and `case` (as written, upper,
lower, sentence). Splitting order: sentence end, clause boundary, then the word nearest the middle, until a chunk
passes all limits. The line-fit check measures the chunk with the renderer's text layout for the chosen preset and
canvas size; glyph advances are cached per preset. A gap pass extends a caption to the next one when the gap is at most
`maxGap`; on overlap, the shorter caption gives up the overlap.
Alternatives: character count only (wrong for proportional fonts and CJK); a separate layout estimate in the UI
(differs from export).

### D5. Subtitle files
Own parser and writer. Parser: blank-line separated blocks, optional index or cue id, times as `HH:MM:SS.mmm` or
`MM:SS.mmm` with `.` or `,`, markup tags removed, HTML entities decoded. Encoding: BOM sniffing, then UTF-8, then a
Windows-1252 fallback. Import keeps the file's timing and creates caption items without word timings on a new caption
track, as one revision. Export writes SRT or WebVTT from a caption track, optionally with word timestamps in WebVTT.
Alternatives: a third-party subtitle crate (licence and maintenance risk for a small format).

### D6. Gallery previews from the renderer
The Captions panel shows each preset as a short looping preview rendered by `ove-render` and cached per preset and
canvas aspect. CSS previews are removed.
Alternative: keep CSS previews (they drift from the export).

### D7. Per-word styles
Caption word timings gain an optional `style` with `color`, `scale` and `emphasis`. It prints in `timeline.tsx` and is
typed in `timeline.d.ts`. Word animations still apply on top. The Inspector lets the person click a word to style it.
Alternative: split a caption into several text items per styled word (breaks timing and editing).

### D8. Translation through the agent
`translate_captions {track, language}` asks the connected agent (in-app or external) for the text of each caption id
as strict JSON. The editor checks that the id set matches, retries once on a mismatch, then writes a new caption track
with `language` set and the same item timings. Word timings are spread by character share so word animations still
run. All of it is one revision.
Alternatives: a bundled translation model (permissive high-quality models are large; common ones are non-commercial).

## Risks / Trade-offs

- [Speakers merged or split wrongly] → threshold setting, manual merge and split, and corrections stored per media;
  detect with a diarization error rate (DER) fixture in CI.
- [Embedding speed on CPU] → measured; target at most 5% of real time; detect with a benchmark in Results.
- [Weight licence not permissive] → licence check before pinning; the alternative model is kept as a fallback; detect
  with the licence review recorded in this file.
- [Layout cost of the fit check] → cache glyph advances per preset; detect with a 1-hour transcript segmentation
  benchmark.
- [Agent translation drops or changes ids] → id-set validation and one retry, otherwise a named error and no
  revision; detect with a mock-agent test.
- [Schema growth] → all new fields are optional with migration entries; detect with round-trip and migration tests.

## Migration Plan

Additive. New document fields are optional and default to absent; older documents open unchanged. A document saved
with the new fields keeps them through the existing unknown-field round trip in older builds.

## Open Questions

- Waves proceed in the order of the master plan.
- Models download on first use; downloads above 50 MB need explicit consent (the speaker model is below that).
- Diarization model weight licences must be verified before a model is pinned; if neither candidate is permissive,
  this capability waits.
- GPU vendor runtimes that need a licence ruling (DirectML) are off by default; CPU is the fallback.

## Results

To be filled during implementation: DER on the 2-speaker fixture, cross-file identity accuracy, embedding speed per OS,
segmentation time for a 1-hour transcript.
