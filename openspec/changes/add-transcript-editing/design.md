# Design: add-transcript-editing

## Context

See proposal.md (Why). Builds on `captions` (local recognition with word timings, per-bundle transcript cache) and on
`add-edit-planners-and-links` (ripple-delete planner, frame grid). Today the mapping from source words to timeline time
lives in the desktop crate (`captions.rs`), so the CLI and MCP cannot use it. Requirements: specs/transcript-editing.

## Goals / Non-Goals

**Goals:**
- One transcript model and one cut path for the GUI, CLI, MCP and the in-app assistant.
- Cuts that sound clean: no clicks, natural pauses, no lost first or last syllables.
- Every cut is one revision that can be previewed as a `timeline.tsx` diff before it is applied.
- Fully local on both operating systems; no account, no credits.

**Non-Goals:**
- Speaker identity, caption layout, subtitle files, translation (add-speakers-and-caption-tools).
- Re-recognising audio after a cut (cuts reuse the cached source transcript).

## Decisions

### D1. Word ids anchored to the source
Each word gets the id `<mediaId>@<sourceStartMs>`. The id names the media file and the source time of the word, so it
does not change when the timeline changes. Words from overlapping recognition windows that would share an id are
de-duplicated on id plus text. `get_transcript` returns the current `rev` with the words, so an agent can send several
removals against one read.
Alternatives: position in the response (changes after every cut; needs a "transcript changed" heuristic and re-reads);
random ids stored in the document (adds document churn for derived data).

### D2. Shared timeline-transcript module
Move the clip-to-timeline word mapping (trim, speed, reverse) from the desktop crate into a headless module of
`ove-transcribe`. Output rows: word id, text, timeline start and end in seconds, clip id, filler flag. A `segments`
mode groups words into sentences. Results page at 10,000 words.
Alternatives: keep a copy per surface (drift between surfaces, which the spec forbids).

### D3. Cut planning in source time
Selected word runs become source-time ranges per clip. Pause rule: for the gap before and after a run, keep
`min(gap / 2, keep / 2)` on each side, and cap the joined pause at `keep`; `keep` is 60 ms (tight), 150 ms (balanced,
default) or 320 ms (loose). Edges then snap to the quietest 10 ms window within ±40 ms, never crossing into a
neighbouring word. Ranges are converted to timeline time through speed and quantised to the timeline frame grid last.
The ranges go to the ripple-delete planner, which emits ordinary ops; the whole cut is one apply and one revision.
A 10 ms audio fade is set on both sides of each seam.
Alternatives: plan in timeline frames (rounding drift at speeds below 1×); cut at word boundaries with no pause rule
(robotic rhythm); no edge snapping (clicks and clipped consonants).

### D4. Caption items follow cuts
When a cut falls inside a caption item, the planner drops the cut words from its word timings and re-times the rest.
A caption item left with no words is removed. This happens in the same revision as the cut.
Alternative: leave captions stale and lint them (the person must fix every cut by hand).

### D5. Dead air from voice activity, relative to speech level
A voice-activity model produces a speech mask in 32 ms cells, cached in the bundle under the content identity key. The
reference level is the loudness-weighted level of speech cells for that file. A non-speech run counts as dead air when
it is at least `relativeDb` (default 12 dB) below the reference and at least `minPause` long (default 0.5 s, range
0.25–3 s). Padding (default 0.15 s, range 0–0.5 s) is kept on each side, except at file or clip edges.
Alternatives: absolute level threshold (today's `detect_silence`; cuts quiet speakers and keeps loud music beds as
"speech"); recogniser gaps only (misses breaths and noise between words).

### D6. Voice activity runtime
Preferred: the voice-activity support built into the recognition library, if the pinned binding exposes it, because it
adds no runtime. Otherwise run the model through the shared runtime from `add-model-runtime`. Task 1.1 decides by a
test on a fixture.
Alternatives: a second bundled inference runtime only for this model (installer size).

### D7. Fillers
Recognition models tend to leave out disfluencies. A verbatim option seeds the recogniser with disfluent example text;
the option is part of the transcript cache key. Filler candidates are (a) words in a per-language filler list and (b)
voiced gaps: speech cells longer than 0.2 s and at most 1.2 s that no word covers. Context-dependent words ("like",
"you know") are flagged but never selected by default.
Alternatives: cut every listed filler automatically (false cuts of content words).

### D8. Transcript search
Unicode-folded (case and accents) phrase match over word tokens, so hits have word-accurate start and end. Each hit
also lists every timeline use of that source range (clip id, timeline time). Only cached transcripts are searched;
the response says how many media files have no transcript yet.
Alternatives: substring match over segment text (no word times; false hits inside words).

### D9. Dry run and surfaces
Every cutting command takes `apply: false`, which returns the plan summary and the unified diff of `timeline.tsx`
without creating a revision. CLI and MCP contracts follow docs/spec where they exist (`transcribe_media`,
`search_transcript`, `detect_silence`); new tools (`get_transcript`, `remove_words`, `remove_silence`) use the same
naming and JSON style. Stale `rev` values are rebased by the normal merge; a word id that no longer appears on the
timeline is reported, not guessed.

### D10. GPU recognition
Enable the recognition library's Metal build on macOS and its Vulkan build on Windows, with a runtime probe and CPU
fallback. Add an "accurate" model option (large, quantised, about 550 MB) behind the download consent rule.

## Risks / Trade-offs

- [Concurrent GUI edits during a cut] → planner ops rebase like any other edit; property tests over random documents;
  detect with the merge-conflict counter in the apply report.
- [Vulkan build fails on some Windows drivers] → runtime probe falls back to CPU; detect with a startup log line and a
  CI job that runs the CPU path.
- [Voice activity misclassifies music with vocals as speech] → relative-level rule plus padding; detect with the music
  bed fixture in tests and precision/recall on a labelled interview.
- [Snapping moves a cut into a breath] → snap window limited to ±40 ms and never past a neighbouring word; detect with
  the seam-level test.
- [Large transcripts make responses heavy for agents] → paging at 10,000 words and a segments mode; detect with a
  response-size test.

## Migration Plan

Additive. Existing transcript caches stay valid; the cache key gains the content identity and the verbatim flag, and
old entries are re-used when the identity matches. No document schema change: cuts are ordinary item ops.

## Open Questions

- Waves proceed in the order of the master plan; this change starts after its dependencies land.
- Models and runtimes download on first use; downloads above 50 MB (the "accurate" model) need explicit consent.
- GPU vendor runtimes that need a licence ruling (DirectML) are off by default; CPU is the fallback.
- Which recognition model is the default on low-end Windows machines: decided by the realtime-factor measurements in
  task 1.2.

## Results

To be filled during implementation: realtime factor per model and OS, cut-planning time for a 1-hour interview,
voice-activity precision and recall, seam peak levels.
