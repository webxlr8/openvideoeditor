# Tasks: add-semantic-media-search

## 1. Model

- [ ] 1.1 Confirm the weight and tokenizer licences at a pinned revision and record them in design.md; verify the
      owner signs off.
- [ ] 1.2 Export the image encoder (half precision and 8-bit) and the 8-bit text encoder with our own scripts, add
      their manifest to `ove-models`; verify cosine ≥ 0.99 against the reference implementation on fixtures for every
      runtime path on both OSes.
- [ ] 1.3 Implement text tokenisation and image preprocessing in crate `ove-vision`; verify golden token ids and
      preprocessed tensors against the reference.

## 2. Sampling and index

- [ ] 2.1 Implement the one-pass shot sampler (cut scores, per-shot points, sharpest-frame choice, black/flat skip,
      256 px decode); verify on a fixture of concatenated clips that boundaries are within ±2 frames and sample counts
      follow the per-shot rule.
- [ ] 2.2 Implement the index file (write, memory-mapped read, corruption detection) keyed by content identity; verify
      tests: relink keeps the index, a damaged file is discarded and rebuilt.
- [ ] 2.3 Register indexing as a P3 scheduled task that calls the model worker; let scene detection answer from the
      cached cut list; verify scene detection results are unchanged on the existing fixtures.

## 3. Ranking and spoken search

- [ ] 3.1 Implement best-frame-per-shot ranking, calibrated confidence, suggested windows and "more like this"; verify
      recall@5/@10 and MRR on the labelled set and record them in design.md Results.
- [ ] 3.2 Implement folded, phrase-aware, BM25-ranked transcript search with padded word ranges; verify golden tests
      for accents, punctuation and phrases that cross segment boundaries.
- [ ] 3.3 Measure query latency at 10,000 and 100,000 rows on both OSes; verify it is ≤ 50 ms at 100,000 rows.

## 4. Organisation

- [ ] 4.1 Add `folder`, `tags`, `subclips` on media and `folders` (including smart folders) on the project to the schema
      with migration tests; print folder paths in the projection header; verify round-trip and merge property tests.
- [ ] 4.2 Implement the organise batch as one revision; verify one undo restores the previous state and concurrent
      folder rename plus tagging keeps both changes.

## 5. Tools

- [ ] 5.1 Add `ove media search|transcript|overview|organize` (headless, `--json`); verify CLI tests for success,
      model-not-installed and closed-project paths.
- [ ] 5.2 Add MCP `search_media`, `media_transcript`, `inspect_media` (overview) and `organize_media`; verify an MCP
      contract test where a hit's clip fragment is pasted into `timeline.tsx` and applies cleanly.
- [ ] 5.3 Document the commands and tools in `docs/cli/media-search.md`; verify the examples run in the CLI tests.

## 6. Media panel

- [ ] 6.1 Move the media panel to `ui/src/media/` with the search field, sections, scope chips and index status pill;
      verify UI tests for each section and the empty and not-installed states.
- [ ] 6.2 Add drag-to-timeline for Moment and Spoken results and "Save as subclip"; verify UI tests that a drop creates
      one revision with the result's range.

## 7. Integration checks

- [ ] 7.1 In the built app on macOS and Windows: install the model with consent, index 10 hours of footage, search,
      drag results; verify results appear within 300 ms and record indexing throughput and worker peak RSS.
- [ ] 7.2 Run an agent session over MCP on both OSes that searches, pastes hits into `timeline.tsx` and organises media;
      verify every step is one revision and the project opens correctly afterwards.
