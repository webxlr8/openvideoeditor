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
**Amended during apply (2026-11-27):** this machine has no Python, so the export scripts could not run here. The
manifest pins an existing ONNX conversion of the released weights at a fixed commit instead, checked file by file
(size and SHA-256). Its 8-bit encoders failed the parity gate against the same conversion's full-precision graphs
(image cosine 0.83–0.93, text 0.96–0.97), so the shipped pair is the half-precision image encoder (cosine 1.0000)
and the text encoder with 4-bit block-quantised weights (cosine ≥ 0.996). The download is 0.63 GB (186 MB + 443 MB +
4 MB tokenizer), not 0.45 GB. Our own export with selective quantisation stays open (tasks.md 1.2) and could bring the
size back down.
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
**Amended during apply (2026-11-27):** the fields sit on the media item itself (`folder`, `tags`, `subclips`, plus a
display `name` for renames), not inside `data`. `updateMedia` sets whole top-level fields, so a change inside `data`
would replace the probe results too and would clash with any concurrent edit of them; top-level fields let a folder
rename and a tagging run concurrently and both stay. A rename changes the projection's media name, so `src` names
follow it. Folders get a header column only when some media is in a folder, so existing projections print unchanged.

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

## Model and tokenizer licences

Checked 2026-11-27 at the pinned revisions (tasks.md 1.1). Owner sign-off is still needed before release.

| File | Source (fixed commit) | Licence found | Notes |
|---|---|---|---|
| `vision_model_fp16.onnx`, `text_model_q4f16.onnx` | ONNX conversion repository `onnx-community/siglip2-base-patch16-256-ONNX` at `d1114256522a37ffa257a0a58017348ab0058db2` | None stated in that repository; its card names `google/siglip2-base-patch16-256` as the base model | A format conversion (and quantisation) of the Apache-2.0 weights below, so the weights' licence applies |
| Weights (source of the conversion) | `google/siglip2-base-patch16-256` at `3f9f96cb90da5dbc758b01813f2f6f1aee24c1ab` | Apache-2.0 (model card `license: apache-2.0`, published by the model's authors) | The research code repository's release notes put "other materials" under CC-BY 4.0; the published model release itself states Apache-2.0 |
| `tokenizer.model` | `google/siglip2-base-patch16-256` at `3f9f96cb90da5dbc758b01813f2f6f1aee24c1ab` (same bytes in the conversion repository) | Apache-2.0, as part of that release | The vocabulary is the one of a larger language-model family whose own releases use custom terms; this copy is distributed in the Apache-2.0 release |

Conclusion: Apache-2.0 is allowed by openspec/config.yaml, and the files are downloaded on the person's request only
(nothing ships in the installer). Two points need the owner's ruling: (1) the conversion repository states no
licence of its own; (2) the tokenizer's vocabulary originates in a model family with custom terms, although this copy
is published under Apache-2.0. If either is refused, our own export (tasks.md 1.2) from the Apache-2.0 release
removes point 1, and point 2 would need a different model.

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
- The model downloads on first use only after consent (0.63 GB as pinned, over the 50 MB limit).
- Accelerated execution on Windows (DirectML) is off by default with the CPU path as fallback until the owner rules on
  its licence.
- Verify that the SigLIP 2 weights and the tokenizer files at the pinned revision are under a licence allowed by
  openspec/config.yaml before pinning them. Checked: see "Model and tokenizer licences"; two points await the owner.
- The labelled evaluation set should come from the owner's own footage; the owner needs to provide or approve it.

## Results

Measured on the Windows 11 development machine (32-core x64, CPU execution, ONNX Runtime 1.32.0). macOS rows are
open (owner checklist).

### Model parity (tasks.md 1.2, 1.3)

`cargo test -p ove-vision --test model -- --ignored` with the model files and the runtime library. The reference is
the full-precision graph of the same conversion (`vision_model.onnx`, `text_model.onnx`), not the original framework
(no Python here). Fixtures: 7 synthetic pictures and 4 text queries.

| Candidate (image / text) | Worst image cosine | Worst text cosine | Image encoder time | Gate |
|---|---|---|---|---|
| 8-bit dynamic (`_quantized`) / 8-bit | 0.8324 | 0.9573 | 154 ms per image | fails |
| 8-bit (`_int8`) / 8-bit | 0.8437 | 0.9573 | 223 ms per image | fails |
| 4-bit (`_q4`) / 4-bit + f16 | 0.9550 | 0.9961 | 295 ms per image | fails (image) |
| 4-bit + f16 / 4-bit + f16 | 0.9551 | 0.9961 | 328 ms per image | fails (image) |
| **half precision / 4-bit + f16 (shipped)** | **1.0000** | **0.9961** | 362 ms per image | passes |
| full precision / full precision | 1.0000 | 1.0000 | 273 ms per image | reference |

Times are per image in a batch of 7 with 4 intra-op threads. Tokeniser: all 11 golden texts give the reference
tokenizer's ids (an independent implementation reading the release's `tokenizer.json`); 60–75 µs per query. One
difference: for a text longer than 63 tokens the reference cut the end-of-sequence id off, while ours keeps it, as the
original pipeline and the Python tokenizer do (truncation counts the special token). Zero-shot sanity with the shipped
pair: each of 5 queries ranks its own picture first, confidence 0.80–0.99, every other picture ≤ 0.05.

Image preprocessing (resize to 256 × 256 with an antialiased triangle filter, then (x − 0.5) / 0.5) is verified by
unit tests against hand-computed values only. A comparison with the original framework's preprocessing needs Python
and stays open.

The model worker path (`ove model-worker`, `cargo test -p ove-cli --test media_search_model -- --ignored`) gives the
in-process embeddings (cosine > 0.9999 on 10 images and a text). Worker timing: 10 images in 9.6 s including the
model load, 5.0 s warm (about 500 ms per image, CPU, 4 threads).

### Sampling and index (tasks.md 2.1–2.3)

- Fixture: 5 concatenated lavfi clips, 20 s at 25 fps, 320 × 180, true cuts at 3.2, 12.0, 13.4 and 14.4 s
  (`cargo test -p ove-vision --features media --test sampler`). Reported cuts: 3.2, 12.0, 13.4, 14.4 (0 frames off).
  Samples per shot 1, 2, 1, 0 (black shot skipped), 1, as the per-shot rule says.
- First pass at 64 px: 500 frames in 168 ms (about 2,970 frames/s on 320 × 180 H.264). Second pass: 5 samples
  decoded at 256 px in 87 ms.
- Non-reference frames are not skipped: the media layer has no option for it, and the full 64 px pass already runs at
  about 100 × real time on this fixture. The cut list is therefore exactly the one scene detection computes (test
  `the_cut_list_matches_scene_detection`), and `detect_scenes` at the default threshold answers from a current index.
- The existing scene-detection fixtures (`ove-analyze` tests) pass unchanged after the scoring moved into a shared
  `CutScorer`.
- Index files: relink keeps the index (same content id, no encoder call); a damaged file fails its CRC, is deleted
  and rebuilt. On Windows a mapped index file cannot be replaced, so readers drop their map before a rebuild.
- Still images that the LGPL FFmpeg build cannot decode (PNG, until the FFmpeg rebuild) are read with the PNG reader.

### Ranking and spoken search (tasks.md 3.1–3.3)

Recall harness (`cargo test -p ove-vision --features media --test recall -- --ignored`): 12 synthetic 3-second clips
(test cards, fractals, noisy colour fields, rings, stripes, cellular patterns), one query each, through the real
sampler, index and ranking. This set is far easier than real footage and only proves the harness; the labelled set of
the owner's footage is still needed (`OVE_RECALL_SET` runs the harness on it).

| Model | recall@5 | recall@10 | MRR |
|---|---|---|---|
| Shipped (half-precision image, 4-bit text) | 1.000 | 1.000 | 0.903–0.944 |
| Full precision | 1.000 | 1.000 | 0.903–0.944 |

The two models give the same ranks; MRR moves between runs because the noise in the colour-field clips is random.
Text confidence for the right clip: 0.17–0.98. "More like this": every clip's own frame ranks its shot first. Unrelated
synthetic pictures still reach image–image cosines of 0.78–0.94, so the similarity floor is set at 0.8 until it can be
re-fitted on the labelled set; the text threshold stays at 0.1 for the same reason.

Query latency (`cargo test -p ove-vision --test latency`, median of 7, ove-vision and the f16 conversion built
optimised in test builds as in shipped builds):

| Rows | Index files | Query | Opening the files (map + CRC, once per session) |
|---|---|---|---|
| 10,000 | 10 | 6.4 ms | 7 ms |
| 100,000 | 100 | 18.1 ms | 57 ms |

Spoken search: golden tests for accents, case, punctuation, a quoted phrase across a sentence (segment) boundary,
150 ms padding kept inside the media, and BM25 order (repeated and rarer terms first). 100,000 words: index 94 ms,
query 2 ms. The search reuses the transcript-search folding from `ove-transcribe`; it lives in `ove-vision` because
it ranks segments and pads ranges, which the existing word-match search does not.

### Organisation (tasks.md 4.1, 4.2)

`cargo test -p ove-tools --test organize` and `cargo test -p ove-core --test organisation`:

- A batch moving 20 files into two new folders and renaming 3 is one revision; one undo restores the document
  (content-equal). Folder paths print in the header's media table; renamed media print under their new names.
- Folder rename (person) and tagging (agent), planned on the same base, keep both changes in either apply order.
- Property tests (256 cases per check run, 1,000 by default): any organise batch that plans is valid, invertible and
  survives save and load byte for byte; random folder batches and random tag/subclip batches commute.
- Older documents without the fields load and save unchanged (the existing fixtures and round-trip properties pass);
  wrong types report `value.type`, and subclips report `subclip.range` and `subclip.id.unique`.
