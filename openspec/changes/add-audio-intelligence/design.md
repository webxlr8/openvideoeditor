# Design: add-audio-intelligence

## Context

See proposal.md (Why). Item times are integer microseconds (`crates/ove-core/src/time.rs`), so placement finer than a
frame is already possible. `playbackSpeed` is a float from 0.01 to 100, so a retime of a few parts per million can be
stored. Probe (`crates/ove-media/src/probe.rs`) reads no timecode and no creation time. The mixer is shared by preview
and export. add-audio-mixing-and-monitoring adds the import analysis cache, which already holds an onset envelope per
media. Requirements: specs/audio-intelligence.

## Goals / Non-Goals

**Goals:**
- Sync accurate to 1 ms at the start and 2 ms at the end of a two-hour recording, with drift corrected.
- Beat results agents can use directly in `timeline.tsx` without unit conversion.
- Denoise that works on both operating systems, for video and audio items, and never ships noisy sound silently.

**Non-Goals:**
- Multicam grouping and angle switching.
- Separation of voice, music and effects into stems.
- Any cloud processing.

## Decisions

### D1. Timecode from container metadata, kept as a rational value
Probe reads the stream or format `timecode` tag (QuickTime timecode track) and, for WAV files, the Broadcast WAV
`time_reference` sample count and iXML `TIMECODE_RATE` and `TIMECODE_FLAG`, through a small chunk reader of our own.
The media entry gains `sourceTimecode {frames, rateNum, rateDen, dropFrame}`. The aligned position of two items is the
difference of their wall-clock start times, divided by the reference item's speed. Alternatives: parse timecode from
file names (unreliable); use file creation dates (device clocks are often wrong by minutes; kept at most as a weak
hint).

### D2. Envelope: band-limited onsets
Sync correlates onset envelopes, not raw samples: mono, band-passed 300 Hz to 4 kHz, spectral flux at a 2 ms hop. The
import analysis cache stores it. Alternatives: RMS envelope (less robust when two microphones colour the sound
differently); raw waveform correlation over the full length (cost grows with the square of the search range).

### D3. Coarse search by FFT, fine search by phase transform
The coarse step is an FFT cross-correlation over all offsets, normalised by running sums over the overlap so that
partial overlaps compare fairly. Cost is O(N log N), so no multi-level pyramid is needed. The fine step runs a
generalised cross-correlation with phase transform on band-passed raw audio within ±20 ms of the coarse peak, then
parabolic interpolation, for sub-millisecond offsets. A match needs at least 3 s of overlap. It is refused when the
ratio of the best to the second-best peak is below a threshold, which guards against repeated music. Alternatives:
brute-force normalised correlation per offset with a pyramid (more code, slower, coarse peaks can be lost between
pyramid levels); onset-time matching (fails on continuous speech).

### D4. Drift: robust line fit, correction on the recorder
For recordings longer than 150 s, offsets are measured in windows every 30 s near the coarse offset and fitted with a
Theil–Sen line, which ignores outlier windows. Slopes between 20 and 500 ppm are accepted; outside that range the
drift is reported but not applied. The residual RMS in ms is reported. The correction changes the playback speed of
the audio-only item (the recorder), never the camera picture, so the picture keeps its frame timing. Alternatives:
least-squares fit (one bad window skews it); retime the camera together with its audio (changes picture timing and
can cause visible frame repeats).

### D5. Placement rules
The most confident match is placed first; placed items become extra anchors for the rest. Matches below the
confidence threshold are skipped and reported. If any item would start before 0, all synced items move right by the
same amount. Moves that would overlap another item on the same track are refused with the item named. The whole plan
is one `apply` batch, so one revision. `--dry-run` returns offsets, confidence, method (timecode or audio), drift and
refusals without editing.

### D6. Beats: a small beat-tracking model through the shared runtime
We export the Beat This "small" checkpoint to ONNX ourselves, with the log-mel frontend in the graph, and run it
through the runtime from add-model-runtime. Audio is resampled to 22,050 Hz mono and processed in 30 s chunks with
overlap; border frames are discarded. A frame is a beat when its probability is at least 0.5 and a local maximum.
Downbeats come from the second output. Bars and the time signature come from downbeat spacing. Tempo is reported as a
curve (median inter-beat interval per 8 s window) and as one median value. Results are cached in
`cache/beats/<key>.json` and never written to the document. Alternatives: classic onset and tempo tracking without a
model (weak on downbeats and on music without strong percussion); a larger model (more accurate on some genres, but
several times the size and run time).

### D7. Beats in timeline time, and as snap targets
`ove audio beats` on an item returns beat times in source seconds and in timeline seconds, mapped through trim, speed
and reverse, so agents can paste them into `timeline.tsx`. The snap engine in `ove-edit` (add-edit-planners-and-links)
receives beat targets from this change; downbeats rank above beats. The timeline draws beat ticks on music clips when
"Snap to beats" is on. Alternative: return source seconds only (agents then convert by hand, which is wrong for
trimmed or retimed clips).

### D8. Denoise: native enhancement library, cached render, aligned blend
The `denoise` field (0–100, default 0, printed as `denoise={70}`) is added to video and audio items. A background task
renders the cleaned sound of the media once with the DeepFilterNet3 model through its native Rust library at 48 kHz
and writes `cache/audio/<key>_dfn3.flac`. The model's look-ahead delay is removed when writing, so clean and original
are aligned to the sample. The mixer opens the clean file as a second source and mixes original × (1 − a) + clean × a.
Until the render is ready the item plays the original sound and shows a progress chip. Alternatives: real-time
denoise in the mixer (risk of audio dropouts on slow machines and during scrub); bake into a new media file (loses the
blend amount and doubles library entries); a cloud provider (costs money, needs the network, stays in
add-ai-generation as an option).

### D9. Export rule for denoise
Export renders any missing clean copy first, with progress. If a render fails, export is refused and names the item,
unless the person passes "allow unprocessed sound" (`--allow-unprocessed`). Alternative: fall back to the original
sound silently (a deliverable would ship with noise the person removed in preview).

### D10. Surfaces
- CLI: `ove audio sync <project> --ref <item> --targets <items> --mode auto|timecode|audio --window <s>
  --min-confidence <0..1> --drift auto|off [--dry-run]`; `ove audio beats <project> <media|item> [--from --to]`;
  `ove audio denoise <project> <items> --amount <0..100>|--off`.
- MCP: `sync_items`, `detect_beats`, `denoise_audio`.
- UI: timeline context command "Synchronise selection" with a result summary; "Snap to beats" toggle; Inspector
  denoise slider with a progress chip.

## Risks / Trade-offs

- [Multichannel recorder files have one microphone per channel] → correlate the sum of channels by default, with an
  option to choose one channel. Detect: a polyphonic WAV fixture in the sync tests.
- [Repeated content gives several similar peaks] → consensus over windows and the best-to-second-best peak ratio
  check; refuse when ambiguous. Detect: a looped-music fixture must be refused, not misplaced.
- [Inspector rounds the speed and loses the drift correction] → the Inspector shows "drift +42 ppm" and keeps the
  exact value unless the person changes speed. Detect: a UI test opens a drift-corrected item and checks the stored
  speed is unchanged after closing the Inspector.
- [Our mel frontend differs from the training frontend] → parity gate on fixtures: logits within 1e-3, beat
  positions identical within one frame. Detect: the parity test in CI.
- [Denoise cache size] → FLAC storage and eviction of entries unused for 30 days; Settings → Storage shows the size.
  Detect: a size assertion on a one-hour fixture.
- [Inference crate versions conflict with other crates] → pin versions and run cargo-deny duplicate checks. Detect:
  cargo-deny in CI.

## Migration Plan

No document migration. `denoise` is an added sound field with default 0. `sourceTimecode` is added to media entries
when they are next probed; missing values mean "no timecode".

## Open Questions

- Waves proceed in master-plan order (default); confirm priority against the open R1 and R2 performance gates.
- Runtimes and models download on first use, with explicit consent above 50 MB. The beat model (about 7 MB) and the
  denoise model (a few MB) are below that limit but are listed in Settings → Storage.
- DirectML stays off by default with a CPU fallback, pending a licence ruling.
- SVT-AV1 stays excluded pending a licence ruling (not used by this change).
- Verify that the Beat This checkpoints, not only its code, are under a permissive licence before pinning.
- Verify that the DeepFilterNet3 weights carry the same permissive licence as the library before pinning.

## Results

To be filled during apply: sync accuracy at head and tail and drift error on fixtures, sync run time for a 2 h
reference against 10 targets, beat accuracy on fixtures, denoise noise reduction and real-time factor, on the macOS
and Windows reference machines.

### Source timecode (group 1, Windows 11 x64)

- `crates/ove-media/tests/timecode.rs` probes fixtures made by the fixture FFmpeg: a 23.976 video at 10:00:00:00
  (864,000 frames, start exactly 864,000 × 1001 / 24,000 s), a 29.97 drop-frame video at 10:00:00;00
  (1,078,920 frames), a 25 fps MP4 at 01:02:03:04, a phone-style MP4 without timecode (no `sourceTimecode`), and
  Broadcast WAV files written chunk by chunk (`bext` with iXML rate, `bext` only, iXML only, zero reference, none).
- A video at 10:00:00:00 (23.976) and a WAV whose `time_reference` is 4 s later are 192,000 samples apart at
  48 kHz, exactly; one more sample in the WAV gives 192,001.
- Deviation from the task text: FFmpeg refuses drop-frame at 23.976 ("drop frame is only allowed with multiples of
  30000/1001"), and drop-frame timecode is not defined for 24-frame rates. The drop-frame path is therefore checked
  at 29.97; the 23.976 fixture is non-drop. A 23.976 label written with `;` is accepted and keeps `dropFrame`, but
  drops no frame numbers.
- A 23.976 recorder 4 real seconds after 10:00:00:00 is labelled 10:00:03:23, not 10:00:04:00: 4 s at 24000/1001 is
  95.9 frames. Sync uses the exact start, not the label.

### Audio sync (group 2, Windows 11 x64, test profile: opt-level 1)

Fixtures are synthetic recordings (`ove_analyze::synth`): speech-like bursts of three partials (300 Hz – 3.5 kHz) in
phrases with pauses, pink noise per microphone, a one-pole low-pass at 4 kHz on the recorder, and a clock rate per
recording. The envelopes go through the same onset code as the import analysis cache (10 ms hops).

| Check | Target | Measured |
| --- | --- | --- |
| Coarse offset, recorder 12.345 s late, 10 dB SNR, low-passed | within 10 ms | 12.3439 s (1.09 ms), confidence 0.64, peak ratio 4.1 |
| Fine offset (GCC-PHAT, 2 s window) on the same fixture | ≤ 1 ms | 0.029 ms |
| Looped music (4 s loop) | refused | refused as ambiguous |
| Two-hour recorder, +50 ppm, 10 dB SNR | drift within 3 ppm | 50.000 ppm (237 of 237 windows, residual 0.167 ms) |
| Same, offset at head / tail of the overlap | ≤ 1 ms / ≤ 2 ms | 0.027 ms / 0.024 ms |
| 10-minute recorder at +2,000 ppm | reported, not applied | 1,985.4 ppm reported, not applied |
| 2 h reference against 10 targets of 10 min, cached envelopes | under 10 s | 4.14 s |
| CLI `ove audio sync --dry-run` on WAV files (7.25 s offset) | ≤ 1 ms | within 1 ms; no revision; apply is one revision |

Notes:
- The coarse search matches the 20 s most active part of each fifth of a long target, not the whole target: with
  2,000 ppm of drift a 60 s probe smeared by 120 ms (12 envelope hops) and no longer correlated. 20 s probes smear by
  at most 40 ms at that rate.
- The import cache keeps a 10 ms onset envelope, not the 2 ms one in D2, and sync reuses it as the task asks. The
  fine step makes up the precision: the 10 ms coarse step lands within ±1 hop and GCC-PHAT searches ±20 ms.
- The fine step and the drift windows read short stretches of raw audio (2 s at the head, 1 s per drift window),
  not the whole file. The 2 h timing uses synthesised audio for those reads, so file decoding time is not in the
  4.14 s; a WAV or AAC read of one second is a few milliseconds.
- The 2 h timing test asserts the 10 s budget with a 2.4× margin at opt-level 1. If it fails under load, it belongs
  in the serial timing lane (docs/flaky-tests.md).
