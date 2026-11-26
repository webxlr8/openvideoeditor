# Design: add-semantic-media-search

## Context

See proposal.md (Why). We have frame-accurate scene detection (`ove-analyze/src/video.rs`, every frame at 64 px),
hardware-first decode (`ove-media`), whisper word-level transcripts cached in the bundle (`ove-transcribe`), and a flat
name filter in the media panel (`ui/src/Editor.tsx`). Model install, consent and the inference worker come from
`add-model-runtime`; scheduler, content identity and skim tiles from `add-background-tasks-and-media-derivatives`.
Requirements: specs/media-search.

## Goals / Non-Goals

**Goals:**
- Useful visual recall on both platforms with no cloud and no account.
- Results that become clips directly, for people and for agents.
- An open, inspectable index that survives relinks and works headless.

**Non-Goals:**
- Meaning-based transcript search (needs a sentence model; later).
- Libraries larger than about 100,000 sampled frames per project (no approximate index needed below that).

## Decisions

### D1. Model: SigLIP 2 base, patch 16, 256 px
An image–text embedding model with 768-dimensional outputs, Apache-2.0 weights (to be confirmed with its tokenizer,
see Open Questions). We export it to ONNX ourselves, with two image-encoder variants (half precision for accelerated
paths, 8-bit for CPU) and an 8-bit text encoder. Release gate: cosine similarity ≥ 0.99 against the reference
implementation on fixtures, for each runtime path. Total download about 0.45 GB, so it always needs consent.
Alternatives: original CLIP ViT-B/32 (weaker on fine detail, older tokenizer); larger SigLIP variants (better recall,
2–4× size and memory); a cloud embedding API (breaks local-first and costs money).

### D2. Shot-aware sampling in one pass
One decode at 64 px with non-reference frames skipped gives cut scores (luma-histogram distance plus mean absolute
difference, as scene detection does today). Per shot: sample the midpoint plus one point every 8 s; at each point pick
the sharpest frame within ±0.5 s (Laplacian variance at 64 px); drop near-black or flat frames using frame statistics;
decode chosen frames at 256 px. The cut list is stored in the index so scene detection can answer from it. A
`SAMPLER_VERSION` constant forces a rebuild when the sampler changes.
Alternatives: fixed-interval sampling with luma-grid change detection (shot ranges only as fine as the interval; pans
make false shots); embedding every frame (10–30× the compute for little recall gain).

### D3. Index format
One file per media at `<bundle>/cache/embeddings/<contentId>.safetensors`: `embeddings` f16 [N, 768] contiguous,
`times` f64 [N], `row_shot` u32 [N], `shots` f64 [S, 2], `cuts` f64 [C]; metadata: model id and hash, sampler version,
preprocessing, duration. Read by memory map. Readable by common numeric tools, so agents and scripts can inspect it.
Alternatives: a custom binary layout with interleaved rows (loads everything into memory); SQLite with blobs (slower
scans, no zero-copy).

### D4. Ranking
Streamed dot products over the mapped matrices (about 20 ms for 100,000 rows, estimated; measured in Results). Each
shot scores by its best frame. Confidence = sigmoid(scale · cosine + bias) using the model's learned calibration;
results below 0.1 confidence are dropped (threshold re-fitted on our labelled set for the 8-bit model). The suggested
window is ±2 s around the best frame, clipped to the shot. Query embeddings are cached on disk by query text.
Alternatives: fixed cosine floor plus a fraction of the top score (hides results for narrow queries); an approximate
nearest-neighbour index (not needed at this scale; adds a dependency and an update path).

### D5. Spoken search
Unicode case and accent folding; `"quoted phrase"` matches consecutive word tokens; BM25 ranks transcript segments;
hit ranges come from word timestamps with 150 ms padding. Runs over cached transcripts only.
Alternative: substring match in segment text with no ranking (imprecise ranges, unordered results).

### D6. Organisation fields
Media registry entries gain `data.folder` (a path string), `data.tags` (strings) and `data.subclips` (`{id, name, in,
out}`); the project gains `folders` (empty folders and smart folders `{path, query, scope}`). These are additive
fields changed through the existing update-media and update-project ops, so no new op kinds and normal 3-way merge.
Folder paths print in the projection header's media table. Schema validation and migration tests cover the new fields.
Alternative: a separate media-manifest file outside the document (would not merge or undo with other edits).

### D7. Tools
`search_media {project, query, scope, media?, limit ≤ 50, similarTo?}` returns hits with `clip` fragments such as
`<clip src="harbour" in={12.4} start={…} end={…} />` (times in seconds per the projection rules; `start` is left for
the agent to set), plus an `index` health block. `media_transcript {project, media, from?, to?}`. `inspect_media
{project, media, overview:true}` builds ≤ 36 tiles from shot list and skim tiles. `organize_media {project, ops[]}` is
one revision. CLI: `ove media search|transcript|overview|organize` with `--json`, headless on bundles. No tool starts a
large model download; it returns `modelNotInstalled` with the size.

### D8. Media panel
New module `ui/src/media/` (moved out of `Editor.tsx`). One search field, 250 ms debounce, sections Moments, Spoken,
Timelines, Files, scope chips, index status pill linking to the Background Tasks popover. Result thumbnails come from
skim tiles through the custom URI protocol. Dragging a result inserts a clip with source in/out set, as one revision.

## Risks / Trade-offs

- Memory: both encoders peak at about 0.6 GB. Mitigation: they run only in the model worker, which exits when idle.
  Detection: editor RSS measured during indexing (must stay within the model-runtime budget).
- Recall of the 8-bit model is lower. Mitigation: re-fit the threshold; ship the half-precision variant where the
  accelerated path exists. Detection: recall@5 and @10 on a labelled set of ≥ 300 shots and 100 queries; the 8-bit
  model must lose ≤ 2 points against full precision.
- Skipping non-reference frames may miss a cut by a frame or two. Detection: the cut-accuracy fixture test (±2 frames).
- Indexing 4K long-GOP media is slow without hardware decode. Mitigation: read proxies when they exist. Detection:
  indexing throughput (frames/s) recorded per machine.
- Same footage in several projects is indexed several times. Accepted for now; a shared store is out of scope.

## Migration Plan

New optional fields only; older builds ignore them. Index files are derived data and can be deleted at any time.

## Open Questions

- Waves proceed in master-roadmap order; this change is wave 3 and needs `add-model-runtime` and
  `add-background-tasks-and-media-derivatives` first.
- The model downloads on first use only after consent (about 0.45 GB, over the 50 MB limit).
- Accelerated execution on Windows (DirectML) is off by default with the CPU path as fallback until the owner rules on
  its licence.
- Verify that the SigLIP 2 weights and the tokenizer files at the pinned revision are under a licence allowed by
  openspec/config.yaml before pinning them.
- The labelled evaluation set should come from the owner's own footage; the owner needs to provide or approve it.

## Results

(Filled in during apply: parity per runtime path, recall@5/@10 and MRR for full and 8-bit models, query latency at
10k and 100k rows, indexing throughput and worker peak RSS on both reference machines.)
