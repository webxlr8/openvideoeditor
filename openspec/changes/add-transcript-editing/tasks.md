# Tasks: add-transcript-editing

## 1. Recognition and timeline transcript

- [x] 1.1 Decide the voice-activity runtime (design.md D6): run the built-in recognition-library path and the shared
      runtime path on the same fixture; verify both give the same speech mask within one 32 ms cell and record the
      decision and timings in design.md Results.
      Done (design.md R1): the shared runtime wins (37 times faster). Every speech run of the runtime path matches a
      built-in run within one cell at both ends; the built-in path also reports one 96 ms run inside the tone bed that
      the runtime path does not (`crates/ove-transcribe/tests/vad.rs`, `--ignored`, passed). Silero weights: MIT.
- [ ] 1.2 Enable GPU recognition builds (Metal on macOS, Vulkan on Windows) with a runtime probe and CPU fallback, and
      add the "accurate" model option behind download consent; verify realtime factor per model on both reference
      machines is recorded in Results and the CPU fallback test passes with the GPU probe forced off.
- [x] 1.3 Move the clip-to-timeline word mapping into a headless `ove-transcribe` module with source-anchored word ids,
      de-duplication of overlapping windows, a segments mode and paging; verify the existing trim/speed test moves into
      the crate and passes, and a property test shows ids are unchanged after random edits elsewhere.
      Done: `ove_transcribe::timeline` (captions use it); tests in that module and `tests/timeline_ids.rs` (500 cases).
- [x] 1.4 Key transcript caches by content identity plus model, language and verbatim flag, re-using old entries when
      the identity matches; verify a renamed media file keeps its transcript and a changed file does not.
      Done: `cache::transcript_key` (verbatim suffix; normal keys unchanged), `cache::lookup`;
      `tests/cache_identity.rs`.
- [x] 1.5 Document the transcript JSON shape in `docs/cli/`; verify the documented examples run in the CLI tests.
      Done: `docs/cli/transcript.md` (`ove transcript`, registry tool `get_transcript`);
      `transcript_examples_and_json_shape` in `crates/ove-cli/tests/docs.rs`.

## 2. Remove words

- [x] 2.1 Implement the cut planner (source-time ranges, pause rule, edge snapping, frame quantisation, seam fades)
      on top of the ripple-delete planner; verify unit tests for each pause setting, for speed below 1×, and for reverse
      clips.
      Done: `ove_transcribe::cuts` (selection, pause rule, quiet-edge snapping on 10 ms levels, one `ove-edit`
      ripple delete with its time map, 10 ms seam fades); `crates/ove-transcribe/tests/cuts.rs`; tool `remove_words`.
- [x] 2.2 Make caption items follow cuts (drop cut words, re-time the rest, remove empty captions) in the same
      revision; verify the caption scenarios in specs/transcript-editing as named tests.
      Done: `caption_shortened_keeps_its_words_aligned`, `a_caption_with_every_word_cut_is_removed`.
- [ ] 2.3 Property test: after a cut, the remaining source words in timeline order equal the transcript minus the
      selected words (10,000 generated cases); verify it passes on both OSes.
      Windows: `remaining_words_are_the_transcript_minus_the_selection` passes (10,000 cases, speeds 0.5–2×, reverse,
      all pause settings). Open until the macOS CI run passes.
- [ ] 2.4 Audio test with the synthetic speech fixture: remove a known word, export, transcribe again; verify the word
      is gone, its neighbours remain, and the seam peak meets the scenario limit.
- [x] 2.5 Refusals: unknown or off-timeline word ids, locked tracks; verify each returns a named error and no revision.
      Done: `refusals_name_the_cause_and_change_nothing` (`transcript.wordMissing` naming the ids, `ripple.blocked`
      naming the item, `request.invalid`); a refusal returns before any op; the MCP test in 5.2 checks the history.

## 3. Dead air and fillers

- [ ] 3.1 Implement speech masks with the chosen runtime, cached in the bundle, and the relative-level dead-air rule
      with minimum pause and padding; verify the silence/music-bed fixture and record precision and recall on a
      labelled 10-minute interview in Results.
      Implemented and the fixture passes (design.md R4): `ove_transcribe::deadair`, streaming `vad::OnnxVad`, masks
      cached in `cache/vad/<contentId>.json`. Open: no labelled 10-minute interview exists on this machine, so
      precision and recall are not measured.
- [x] 3.2 Extend `detect_silence` with a voice-activity mode and add the removal command; verify detection-only leaves
      the document unchanged and removal is one revision.
      Done: `detect_silence` `mode: "speech"`, tool `remove_silence`; `crates/ove-tools/tests/transcript.rs`
      (`detection_only_leaves_the_document_unchanged`, `dead_air_removal_is_one_revision_and_the_preview_is_what_applies`).
- [x] 3.3 Add the verbatim option and filler candidates (word list per language, voiced gaps, context flags); verify a
      recorded fixture with 10 fillers finds at least 8 and selects no content word by default.
      Done: `Options.verbatim`, `deadair::fillers`, tool `find_fillers`; 10 of 10 found, no content word selected
      (`tests/fillers_audio.rs`, `--ignored`, base model; design.md R4).

## 4. Search and dry run

- [ ] 4.1 Implement phrase search with Unicode folding, word-accurate hits, timeline uses and the untranscribed count;
      verify unit tests for accents, punctuation and phrases across caption boundaries, and a 100,000-word search under
      10 ms.
- [ ] 4.2 Implement dry run for every cutting command, returning the plan summary and the `timeline.tsx` diff; verify
      the preview-then-apply scenario.

## 5. Surfaces

- [ ] 5.1 CLI commands `ove transcript`, `ove transcript search`, `ove edit remove-words`, `ove edit remove-silence`
      with `--json` and exit codes, documented in `docs/cli/`; verify CLI integration tests for success and refusal
      paths, run with the app closed and with it open.
- [ ] 5.2 MCP tools `transcribe_media`, `get_transcript`, `search_transcript`, `remove_words`, `remove_silence` and the
      `detect_silence` mode, matching docs/spec/mcp-tools.md where defined; verify an MCP contract test per tool.
- [ ] 5.3 Assistant tools with the same contracts; verify an assistant test where a scripted model removes words using
      ids from one transcript read, without re-reading.
- [ ] 5.4 Transcript panel: playback follow, click to seek, strike-through selection and Delete to cut, "Remove pauses"
      and "Remove fillers" with a preview overlay on clip waveforms; verify UI tests for each action.
- [x] 5.5 Mark add-captions task 1.5 as taken over by this change; verify the add-captions tasks file points here.
      Done: `openspec/changes/add-captions/tasks.md` 1.5 names this change and its tasks.

## 6. Integration checks

- [ ] 6.1 In the built app on macOS and on Windows: transcribe a 10-minute interview, cut words from the panel, remove
      pauses, undo and redo, export; verify the export plays with no clicks at seams and History shows one revision per
      action.
- [ ] 6.2 An external agent over MCP reads the transcript, removes fillers and dead air with dry runs first, then
      applies; verify the edits appear live in the open app and can be undone.
- [ ] 6.3 Performance: cut planning for 200 removals on a 1-hour interview under 1 s on both reference machines; record
      the numbers in design.md Results.
