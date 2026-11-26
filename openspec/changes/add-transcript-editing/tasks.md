# Tasks: add-transcript-editing

## 1. Recognition and timeline transcript

- [ ] 1.1 Decide the voice-activity runtime (design.md D6): run the built-in recognition-library path and the shared
      runtime path on the same fixture; verify both give the same speech mask within one 32 ms cell and record the
      decision and timings in design.md Results.
- [ ] 1.2 Enable GPU recognition builds (Metal on macOS, Vulkan on Windows) with a runtime probe and CPU fallback, and
      add the "accurate" model option behind download consent; verify realtime factor per model on both reference
      machines is recorded in Results and the CPU fallback test passes with the GPU probe forced off.
- [ ] 1.3 Move the clip-to-timeline word mapping into a headless `ove-transcribe` module with source-anchored word ids,
      de-duplication of overlapping windows, a segments mode and paging; verify the existing trim/speed test moves into
      the crate and passes, and a property test shows ids are unchanged after random edits elsewhere.
- [ ] 1.4 Key transcript caches by content identity plus model, language and verbatim flag, re-using old entries when
      the identity matches; verify a renamed media file keeps its transcript and a changed file does not.
- [ ] 1.5 Document the transcript JSON shape in `docs/cli/`; verify the documented examples run in the CLI tests.

## 2. Remove words

- [ ] 2.1 Implement the cut planner (source-time ranges, pause rule, edge snapping, frame quantisation, seam fades)
      on top of the ripple-delete planner; verify unit tests for each pause setting, for speed below 1×, and for reverse
      clips.
- [ ] 2.2 Make caption items follow cuts (drop cut words, re-time the rest, remove empty captions) in the same
      revision; verify the caption scenarios in specs/transcript-editing as named tests.
- [ ] 2.3 Property test: after a cut, the remaining source words in timeline order equal the transcript minus the
      selected words (10,000 generated cases); verify it passes on both OSes.
- [ ] 2.4 Audio test with the synthetic speech fixture: remove a known word, export, transcribe again; verify the word
      is gone, its neighbours remain, and the seam peak meets the scenario limit.
- [ ] 2.5 Refusals: unknown or off-timeline word ids, locked tracks; verify each returns a named error and no revision.

## 3. Dead air and fillers

- [ ] 3.1 Implement speech masks with the chosen runtime, cached in the bundle, and the relative-level dead-air rule
      with minimum pause and padding; verify the silence/music-bed fixture and record precision and recall on a
      labelled 10-minute interview in Results.
- [ ] 3.2 Extend `detect_silence` with a voice-activity mode and add the removal command; verify detection-only leaves
      the document unchanged and removal is one revision.
- [ ] 3.3 Add the verbatim option and filler candidates (word list per language, voiced gaps, context flags); verify a
      recorded fixture with 10 fillers finds at least 8 and selects no content word by default.

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
- [ ] 5.5 Mark add-captions task 1.5 as taken over by this change; verify the add-captions tasks file points here.

## 6. Integration checks

- [ ] 6.1 In the built app on macOS and on Windows: transcribe a 10-minute interview, cut words from the panel, remove
      pauses, undo and redo, export; verify the export plays with no clicks at seams and History shows one revision per
      action.
- [ ] 6.2 An external agent over MCP reads the transcript, removes fillers and dead air with dry runs first, then
      applies; verify the edits appear live in the open app and can be undone.
- [ ] 6.3 Performance: cut planning for 200 removals on a 1-hour interview under 1 s on both reference machines; record
      the numbers in design.md Results.
