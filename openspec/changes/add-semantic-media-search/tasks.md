# Tasks: add-semantic-media-search

## 1. Model

- [ ] 1.1 Confirm the weight and tokenizer licences at a pinned revision and record them in design.md; verify the
      owner signs off.
      _Apply note: licences checked and recorded (design.md "Model and tokenizer licences"); open: owner sign-off on
      two points._
- [ ] 1.2 Export the image encoder (half precision and 8-bit) and the 8-bit text encoder with our own scripts, add
      their manifest to `ove-models`; verify cosine ≥ 0.99 against the reference implementation on fixtures for every
      runtime path on both OSes.
      _Apply note: manifest added (`siglip2-base`), pinning an existing conversion; shipped pair passes ≥ 0.99 against
      the full-precision graph on Windows CPU (design.md Results). Open: our own export scripts (needs Python), parity
      against the original framework, macOS._
- [ ] 1.3 Implement text tokenisation and image preprocessing in crate `ove-vision`; verify golden token ids and
      preprocessed tensors against the reference.
      _Apply note: both implemented; token ids match the reference tokenizer (11 golden texts). Open: preprocessed
      tensors against the original framework (needs Python); unit tests check hand-computed values._

## 2. Sampling and index

- [x] 2.1 Implement the one-pass shot sampler (cut scores, per-shot points, sharpest-frame choice, black/flat skip,
      256 px decode); verify on a fixture of concatenated clips that boundaries are within ±2 frames and sample counts
      follow the per-shot rule.
- [x] 2.2 Implement the index file (write, memory-mapped read, corruption detection) keyed by content identity; verify
      tests: relink keeps the index, a damaged file is discarded and rebuilt.
- [x] 2.3 Register indexing as a P3 scheduled task that calls the model worker; let scene detection answer from the
      cached cut list; verify scene detection results are unchanged on the existing fixtures.

## 3. Ranking and spoken search

- [ ] 3.1 Implement best-frame-per-shot ranking, calibrated confidence, suggested windows and "more like this"; verify
      recall@5/@10 and MRR on the labelled set and record them in design.md Results.
      _Apply note: implemented; the harness runs on a 12-clip synthetic set (design.md Results). Open: the labelled
      set of the owner's footage (≥ 300 shots, 100 queries) and the threshold re-fit on it._
- [x] 3.2 Implement folded, phrase-aware, BM25-ranked transcript search with padded word ranges; verify golden tests
      for accents, punctuation and phrases that cross segment boundaries.
- [ ] 3.3 Measure query latency at 10,000 and 100,000 rows on both OSes; verify it is ≤ 50 ms at 100,000 rows.
      _Apply note: Windows measured (18 ms at 100,000 rows, design.md Results). Open: macOS._

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
