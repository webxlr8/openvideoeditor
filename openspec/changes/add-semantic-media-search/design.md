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
An image–text embedding model with 768-dimensional outputs and Apache-2.0 weights (licence review below). The pinned
files are an existing ONNX conversion of the released weights at a fixed commit, checked file by file (size and
SHA-256): the image encoder in half precision and the text encoder with 4-bit block-quantised weights, plus the
release's tokenizer. Total download 0.63 GB (186 MB + 443 MB + 4 MB), so it always needs consent; the consent prompt,
Settings > Storage, the media panel and `ove models` show "0.63 GB", the model's name (SigLIP 2 base) and its licence.
Release gate: cosine similarity ≥ 0.99 against the reference implementation on fixtures, for each runtime path.
**Approved during apply (2026-11-27, CTO and CEO, with conditions):** the plan was our own export with 8-bit encoders.
This machine has no Python, so the conversion above is pinned instead. Its 8-bit encoders failed the parity gate
against the same conversion's full-precision graphs (image cosine 0.83–0.93, text 0.96–0.97); the shipped pair passes
(image 1.0000, text ≥ 0.996, design.md Results). Our own export with selective quantisation, and parity against the
original framework, stay open (tasks.md 1.2, needs Python); they could make the download smaller.
**Release is blocked on the owner's licence sign-off** (tasks.md 1.1, the two points under "Model and tokenizer
licences"). Until then the model stays in the catalog for testing, downloads only on the person's request, and ships
in no installer.
**Found later during apply (parity against the original framework, Results):** the shipped text encoder fails the
gate on one fixture text (cosine 0.9360). Our own export passes at 0.56 GB, and the conversion's half-precision pair
passes at 0.76 GB; switching the manifest to either needs approval (and, for ours, an immutable hosting location).
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

### Tools (tasks.md 5.1–5.3)

`cargo test -p ove-cli --test media_search` (headless `ove` on a closed project, no app) and
`cargo test -p ove-tools --features media --test overview`:

- `ove media search` returns spoken hits with padded ranges and clip lines; with an empty model folder Moments is empty,
  `index.visual` is `notInstalled` with the 0.63 GB size, and the model folder stays empty.
- "More like this" (`--like MEDIA@SECONDS`, `similarTo`) works from the index files alone, also without the model;
  the source shot ranks first and an unrelated shot falls below the similarity floor.
- Over MCP, a Moments hit's and a Spoken hit's `clip` line pasted after a `<track>` line of `timeline.tsx` applies
  with `apply_timeline`, and the new clip's `in` and length are the hit's range.
- `inspect_media` with `overview` on a 10-minute two-shot video: one sheet of 36 labelled tiles, both shots on it,
  in 310 ms (seeks to the sampled times; no full decode).
- The 8 examples in `docs/cli/media-search.md` run with their documented exit codes.
- Text queries embed through the shared `ove model-worker` and are cached on disk per query text
  (`cache/embeddings/queries/`); the worker program is the `ove` binary itself, or the `ove` beside the app.

### Media panel (tasks.md 6.1, 6.2)

`ui/src/media/MediaPanel.tsx` replaces the media branch of the side panel; `npx pnpm@10.28.0 test` (36 files, 210
tests) passes, 5 of them in `ui/src/media/MediaPanel.test.tsx`:

- One search after typing stops (250 ms); Moments, Spoken, Timelines and Files sections; scope chips search one
  section; clearing the field shows the media again.
- "No visual matches" when no shot passes the threshold while Spoken still lists hits; untranscribed files are counted
  with a Transcribe link; with the model missing, Moments and the pill say so with the size (0.63 GB) and the pill
  offers the download, which starts only after the person confirms it (the runtime first when it is missing).
- "Indexing n of N" while indexing; the pill opens the Background Tasks popover.
- Dropping a Moment on the Main lane plans one `overwrite` (one revision) with `trimStart` = the result's in and the
  window's length; media cards and subclips drag the same way.
- "Save as subclip" is one `organize_media` revision, and the subclip then shows under its media.
- The app commands run the same registry tools: `media_search` (off the project turn, so a first query that starts the
  model worker does not hold up editing), `media_organize` (one revision) and `media_index_status` (which also queues
  indexing when the model was installed after the project opened).

Not measured here: results within 300 ms on 10 hours of indexed footage in the built app (tasks.md 7.1).

### Conditions of the model approval (CTO and CEO, 2026-11-27)

- Size, name and licence: the consent message (`model.consent` and "model not installed" results), `ove models list`,
  the first download progress line, Settings > Storage and the media panel's install offer show "0.63 GB",
  "SigLIP 2 base" and "Apache-2.0" (tests: `ove-vision` `model::tests`, `ove-models --test siglip2`,
  `ove-cli --test media_search` `ove_models_shows_the_size_name_and_licence`, UI `MediaPanel.test.tsx` and
  `ModelsSection.test.tsx`). Sizes from 500 MB up read in gigabytes everywhere.
- Install, resume, hash, remove (`cargo test -p ove-models --test siglip2`, stand-in bodies under the real id, version,
  file names and licence): agents and features get `NeedsConsent` and nothing is requested; the person's install
  resumes the text encoder from 120,000 bytes after a dropped connection; a damaged tokenizer is refused by its
  SHA-256 and nothing is installed; Remove frees the files and the model reads as not installed.
- Without the model, Spoken, Timelines and Files still answer (CLI test on a closed project with an empty model folder;
  UI test with the not-installed result), and nothing is downloaded.
- Task 1.2 stays open (see below: the shipped text encoder fails against the original framework). Release is
  blocked on the owner's licence sign-off (task 1.1).

### Parity against the original framework (tasks.md 1.2, 1.3)

Reference: the released checkpoint `google/siglip2-base-patch16-256` at `3f9f96cb90da5dbc758b01813f2f6f1aee24c1ab`,
loaded by transformers 5.24.1 on torch 2.15.0 (CPU) in the dev venv, installed by hash
(`tools/vision-export/requirements.txt`). Before loading, `model.safetensors` matched its pinned SHA-256
`6125cacc…5dc863`, as did `tokenizer.json` and `tokenizer.model`; weights only, offline, no remote code. The
reference's logit scale and bias (112.90118, −16.771803) equal the constants in `ove-vision`. Fixtures: 8 generated
pictures (colour fields, stripes, checkerboard, gradient, disc, noise, a portrait) and 8 texts, written with the
reference outputs to `crates/ove-vision/tests/reference/` by `tools/vision-export/siglip2_export.py`.

Our preprocessing and tokenizer (`cargo test -p ove-vision --test reference`, the first test runs in every test run):
token ids equal the reference's for all 8 texts, including a text cut at 64 tokens (the end-of-sequence id is kept,
as our tokenizer does). The sampled preprocessed tensors differ by at most one 8-bit level (0.0078); channel means
differ by at most 0.0004.

Encoders, worst cosine against the reference embeddings (ONNX Runtime 1.32.0, CPU):

| Encoder | Size | Reference inputs (Python) | Our preprocessing and tokens (Rust) | Gate |
|---|---|---|---|---|
| Shipped image, half precision | 186 MB | 1.0000 | 0.9996 | passes |
| Shipped text, 4-bit + f16 | 443 MB | 0.9360 ("The Price Is Right!"; others 0.9917–0.9977) | 0.9360 | **fails** |
| Conversion's text, half precision (pinned commit, not shipped) | 565 MB | 1.0000 | — | passes |
| Our image, full precision | 372 MB | 1.0000 | — | passes |
| Our image, half precision | 186 MB | 1.0000 | 0.9996 (with our text below) | passes |
| Our image, 8-bit (matrix products only) | 102 MB | 0.8655 | — | fails |
| Our text, full precision | 1,130 MB | 1.0000 | — | passes |
| Our text, 8-bit (matrix products and embedding) | 284 MB | 0.8872 | — | fails |
| Our text, 8-bit embedding table only | 540 MB | 0.9917 | — | passes |
| **Our text, 8-bit embedding table, half-precision layers** | **368 MB** | **0.9917** | **0.9917** | **passes** |

The earlier parity check (against the conversion's own full-precision graphs, on short texts only) missed the shipped
text encoder's failure: a capitalised, punctuated text loses most in 4 bits. Two pairs pass on Windows CPU:

- our half-precision image encoder and our text encoder with an 8-bit embedding table: 0.56 GB (186 + 368 + 4 MB),
  smaller than the shipped 0.63 GB, but our exports are not hosted anywhere yet; the manifest needs an immutable
  location (for example a versioned release asset of this repository);
- the conversion's half-precision image and text encoders, already at the pinned commit: 0.76 GB.

Until one of them is approved and pinned, the shipped text encoder does not meet the release gate, and task 1.2 stays
open (macOS is open as well). Task 1.3 is done.
