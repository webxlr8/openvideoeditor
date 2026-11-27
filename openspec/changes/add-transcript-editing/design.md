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

### R1. Voice-activity runtime (task 1.1, Windows 11 x64, Ryzen 9 5950X)

Fixture (made on this machine with the system speech synthesiser and FFmpeg; not committed): 20.9 s at 16 kHz mono,
speech, 2 s of digital silence, speech, 4 s of two sine tones mixed (a stand-in music bed), speech. Model: Silero
VAD 6.x in both forms, one thread, test profile (optimised dependencies).

| Path | Model file | Time for 20.9 s | Real-time factor | Speech runs found |
|---|---|---|---|---|
| Recognition library, built-in voice activity | `ggml-silero-v6.2.0.bin`, 885,098 bytes | 4.8–5.5 s | 0.23–0.26 | 7 |
| Shared runtime (ONNX Runtime 1.32.0, `ove-models`) | `silero_vad.onnx`, 2,204,103 bytes | 129–149 ms (+128–187 ms to load the library and model) | 0.006–0.007 | 6 |

- Agreement: each of the 6 runs of the runtime path has a run of the built-in path with both ends within one 32 ms
  cell. The built-in path also marks 96 ms (3 cells) inside the tone bed as speech; the runtime path does not.
  Counted over all cells, boundaries moved at most 2 cells because of that extra run.
- Decision (D6): the shared runtime. The built-in path needs no download, but at a real-time factor of about 0.25 it
  would spend about 15 minutes on a one-hour interview, against about 25 s with the runtime; it also marked part of
  the tone bed as speech. Costs of the decision: the runtime library (16.7 MB download on Windows, 22.5 MB on macOS)
  must be installed with consent before dead-air removal works, and the feature reports "runtime not installed"
  until then.
- Pinned model: `silero-vad` in the model catalog, `silero_vad.onnx` from the Silero repository at commit
  `446da2ad95fc90d67a3b12e12ea3f5410b9b3ec4`, SHA-256 `f6f4407b41e05e277e7f5cfd9b9ae957b1dfe763c6076a934734c0b448e09b31`;
  it runs in the editor process (2.2 MB, under the 10 MB in-process limit).
- Weights licence: MIT. Evidence at that commit: `LICENSE` ("MIT License, Copyright (c) 2020-present Silero Team");
  README: "Published under permissive license (MIT)". One README badge has the alt text "CC BY-NC 4.0", but the
  badge image reads MIT and links to the MIT `LICENSE` (same finding as add-model-runtime R1). The ggml conversion
  used for the comparison declares `license: mit` on its model card.

### R2. Timeline transcript (tasks 1.3–1.5)

- The mapping moved from the desktop crate to `ove_transcribe::timeline`; captions, `get_transcript` and
  `ove transcript` use it. `ove-transcribe` now builds without the recognition library (feature `recognize`, on by
  default), so the command line and the tool registry read transcripts without linking whisper.cpp.
- Word ids `<mediaId>@<sourceMs>`; words repeated by overlapping recognition windows (same media, start in ms and
  text) are listed once. Property test: 500 random move/trim/length/speed edits of another clip leave every id of
  the first clip unchanged, and the edited clip's ids stay source ids.
- Cache keys: `<contentId>-<model>-<language>`, plus `-verbatim` for verbatim transcripts; existing keys are
  unchanged, so caches made before this change are used as they are.

### R3. Remove words (group 2, Windows)

- Pause rule as implemented: each side of a cut keeps `min(gap / 2, keep / 2)` of the pause next to the neighbouring
  word; the rest of the gap goes with the word. Measured joined pauses at 30 fps (gaps of 0.4 s): tight 60 ms,
  balanced 150 ms, loose 320 ms, each within one frame of rounding.
- Edges move to the middle of the quietest 10 ms cell within ±40 ms of the computed edge, never into a neighbouring
  word; the levels (10 ms RMS of 16 kHz mono) are cached per media under its content identity
  (`cache/levels/<contentId>.json`). Ranges are then quantised to the frame grid by the ripple-delete planner, so the
  final edges land within half a frame of the quiet point.
- Cuts remove each range from every track holding a cut clip, plus sync-locked tracks and link partners (the
  planner's ripple rule); the planner's time map is recorded with the revision. Caption items are rebuilt from the
  items before the cut, so a sync-locked captions track is not split.
- Property test: 10,000 cases (3–24 words, word lengths 0.12–0.8 s, gaps 0–0.9 s, speeds 0.5–2×, forward and
  reverse, all pause settings) in about 2 s.

### R4. Dead air and fillers (group 3, Windows)

- Rule as implemented: the speech level is the mean power of all speech cells (32 ms, probability ≥ 0.5); a
  non-speech run counts as dead air when it lasts at least the minimum pause and its mean power is at least
  `relativeDb` below the speech level; padding is kept on both sides except at the start and end of the file, and at
  a clip's own edges when cutting the timeline.
- Real-audio fixture (system speech synthesiser; speech, 2 s of digital silence, speech, 4 s of two tones at
  −20.8 LUFS, speech): speech level −19.4 dBFS. Removed: the silence (5.01–7.59 s after padding), the tail before the
  tones (0.47 s of quiet), a 0.6 s pause inside the first sentence and the quiet end; the tones were kept. With the
  tones at −43 dBFS (24 dB below the speech) they are cut, as the rule says.
- Speech mask cost: about 130 ms for 20.9 s of audio (see R1); masks and levels are cached per content identity, so
  a second detection or removal reads only the cache.
- Fillers: the system speech synthesiser read a sentence with 10 hesitations; the base model with the verbatim
  option transcribed all 10 ("hmm" came out as "um"), and only those 10 were selected. Without a labelled interview,
  precision and recall on real conversation are not measured (task 3.1 stays open).

### R5. Surfaces (group 5, Windows)

- Registry tools on the in-app, MCP and pinned surfaces: `get_transcript`, `search_transcript`, `transcribe_media`,
  `remove_words`, `remove_silence`, `find_fillers`, and `detect_silence` with `mode: "speech"`.
- `transcribe_media` links the recognition library only in builds with the `speech` feature (`ove-tools`,
  `ove-cli`); the desktop app enables it. Whether the installer's `ove` command line should include it (it adds
  whisper.cpp to that binary) is an open packaging decision; without it the tool answers `speech.unavailable`.
- Task 5.4 (the Transcript panel) was not started in this session.
