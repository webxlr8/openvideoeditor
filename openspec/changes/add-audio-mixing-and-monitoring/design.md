# Design: add-audio-mixing-and-monitoring

## Context

See proposal.md (Why). Today the mixer (`crates/ove-render/src/audio.rs`) applies a static volume of 0–200 %, fades,
the `audio-fade` transition, speed changes through band-limited resampling, and a −1 dBFS soft-knee limiter on sample
peaks. The same mixer feeds preview (`apps/desktop/src-tauri/src/engine.rs`) and export, so any change here sounds the
same in both. Analysis (`crates/ove-analyze/src/audio.rs`) measures whole files on demand: levels, waveform PNG,
silence, EBU R128. It decodes the file again on every call. `KEYFRAME_PROPERTIES` in `crates/ove-core/src/schema.rs`
has no `volume`. The timeline draws no waveforms. The transport mutes audio at any speed other than 1×. The Duck
button sets every audio clip to 30 %. Requirements: specs/audio-mixing.

## Goals / Non-Goals

**Goals:**
- One mixer for preview, scrub, meters and export, so what people hear, see and deliver is the same signal.
- Every audio edit (keyframe, gain drag, duck) is an ordinary revision, visible in `timeline.tsx` and undoable.
- Decode each media file once for all audio analysis.
- Deliver files at a stated loudness without clipping between samples.

**Non-Goals:**
- Machine-learning analysis (add-audio-intelligence).
- A general keyframe lanes panel (add-markers-keyframe-lanes-and-versions). This change draws only the volume line on
  the clip itself.
- Surround, plug-ins, pitch-preserving time stretch.

## Decisions

### D1. Volume keyframes in percent, interpolated in decibels
Add `volume` to the keyframe property list. Values are percent (0–200), the same unit as the static `volume` field, so
agents see one unit. The effective gain is static volume × keyframe value × fades. The mixer converts each keyframe
value to dB (floor −60 dB, which maps to silence) and interpolates in dB with the track's easing, evaluated once per
32-sample block. Alternatives: interpolate in linear gain (a fade from 100 % to 0 % sounds abrupt at the end, because
most of the audible change happens in the last 10 %); store dB in the document (a second unit next to the static
percent field, confusing for agents); per-sample evaluation (no audible gain over 32-sample blocks at 48 kHz, and
about 30× the cost).

### D2. One import-time analysis pass, cached by media identity
A background task decodes each audio-bearing media file once and writes:
- a min/max peak pyramid per channel: base level of 256 samples per bin, each next level halves the bin count, stored
  as 8-bit values in `cache/waveforms/<key>.peaks` (about 1.4 MB per hour of stereo);
- a loudness summary (integrated LUFS, true peak, loudness range);
- the band-limited onset envelope that add-audio-intelligence uses for sync.
The cache key is the media `contentId` from add-background-tasks-and-media-derivatives. Until that lands, the key is
the existing path + size + mtime hash, and the files move when the key changes. Alternatives: decode per feature (the
current state; each of waveform, loudness, sync and beats decodes the file again); one fixed resolution (zoomed-out
views would read too many bins, zoomed-in views would be blocky); 16-bit values (twice the size with no visible gain
at timeline heights).

### D3. Waveforms drawn in the web view, from a range query
A Tauri command returns peaks for a source range at a requested bins-per-pixel density, choosing the pyramid level
that is at least as fine as needed. The timeline draws only visible clips on a canvas, maps the source range through
trim, speed and reverse, and scales the shape by the clip's gain curve. Alternatives: render PNG strips in Rust (the
current `waveform_png`; every zoom or gain change needs a new image over IPC); draw in the native layer (the timeline
is a web view; mixing native and web drawing adds a second hit-test path).

### D4. Gain line on the clip
The clip shows a horizontal gain line. Dragging the line changes the static volume. Alt-click (Option-click on macOS)
adds a volume keyframe; dragging a point moves it in time and value; Delete removes the selected point. One pointer
gesture produces one revision, the same rule `Timeline.tsx` already uses for trims. Alternative: edit volume
keyframes only in the Inspector (slow, and people expect a line on the clip).

### D5. Meters measured on the producer side
The mixer output passes through a meter tap that computes, per channel, sample peak, RMS, true peak (4× oversampling,
the kernel already in `ove-analyze`) and momentary (400 ms) and short-term (3 s) loudness. Each reading carries the
media time of its block. The app emits readings at most 30 times per second and the UI shows the reading whose media
time matches the heard position (`AudioPosition` already accounts for device latency). Ballistics (fall 24 dB/s, peak
hold 1.5 s, then fall 18 dB/s, clip indicator held until clicked) are a pure TypeScript function with unit tests.
Alternatives: measure in the web view from a copy of the samples (large IPC traffic); measure at the device callback
(the callback must stay real-time safe; loudness filters there risk underruns).

### D6. Scrub grains from the shared mixer
A scrub worker calls the mixer for 1 s windows around the playhead and keeps them in an LRU cache keyed by revision id
and window start. Each scrub event plays one 60 ms grain with a Hann window. A new grain flushes the queued one, so
audio never lags the pointer. The UI throttles scrub events to 60 per second. Because the cache key includes the
revision id, an edit invalidates exactly the stale windows; no debounce is needed. Alternatives: decode source files
directly during scrub (does not include effects, gain or other tracks, so it sounds different from playback); a
tape-style varispeed scrub (needs continuous resampling and sounds poor at very low speeds).

### D7. Ducking writes keyframes, never a hidden effect
`duck` takes music items, dialogue items, a depth in dB, attack, release and hold. Speech spans come from transcript
word timings when a transcript exists (`ove-transcribe` cache), else from a level gate with hysteresis on the 10 ms
windows of the dialogue items. Spans closer than `hold` merge. The tool writes volume keyframes on each music item:
the ramp down starts `attack` before speech, the ramp up ends `release` after it. The curve is simplified to at most
2 keyframes per second (Ramer–Douglas–Peucker). `--merge` multiplies with existing volume keyframes; `--replace`
rewrites keyframes only inside the affected range. One call is one revision. Alternatives: a live sidechain
compressor in the mixer (not visible in `timeline.tsx`, not editable point by point, and agents cannot review it); a
flat level change (the current behaviour, wrong between lines of dialogue).

### D8. Loudness on export: measure, gain, true-peak limit
The BS.1770 core moves from `ove-analyze` to `ove-render/src/loudness.rs`; `ove-analyze` re-exports it, so there is
one implementation. When a loudness preset is set, export runs two passes: pass 1 mixes audio only (no video decode)
and measures integrated loudness; pass 2 applies one static gain before a true-peak look-ahead limiter (ceiling
−1 dBTP, 1.5 ms look-ahead, results independent of block size). The result reports integrated loudness and true peak
before and after. The limiter replaces the current sample-peak soft knee for all exports and preview. Alternatives:
dynamic normalisation (changes the mix balance over time, which editors do not expect); one pass with a running
estimate (the gain would change during the file); FFmpeg `loudnorm` (would put a second loudness implementation in
the export path and does not run in preview).

### D9. Audio-only export and extract audio
Export settings gain an audio kind with format `m4a` (AAC, iPod muxer with an edit list for encoder priming), `wav`
(24-bit PCM) or `flac`. The audio kind skips the video renderer entirely. The settings model itself comes from
extend-export-formats-and-queue; this change adds the audio kind and its muxers. Extract audio decodes a media file's
audio, writes it into `media/` as WAV or FLAC and registers it through an `AddMedia` op. Alternative: export video and
strip the audio track (wastes a full video render).

### D10. Surfaces
- CLI: `ove audio duck <project> --music <items|track> --under <items|track> --depth <dB> --attack <s> --release <s>
  --hold <s> --source transcript|level --merge|--replace [--dry-run]`; `ove media extract-audio <project> <media>
  --format wav|flac`; `ove export --audio-only --format m4a|wav|flac`; `ove export --loudness off|-14|-16|-23|-24`;
  `ove audio loudness <project> --timeline` measures the timeline mix.
- MCP: `duck_audio`, `extract_audio`; `export_video` gains `audioOnly` and `loudness`; `measure_loudness` accepts a
  timeline target.
- UI: waveforms and gain line on clips, meters next to the viewer, Duck popover in the Audio panel, loudness and
  audio-only choices in the export dialog.

## Risks / Trade-offs

- [Re-ducking after edits leaves stale keyframes] → `--replace` limits rewrites to the affected range; the panel shows
  a "dialogue changed since last duck" note when any dialogue item has a newer revision than the duck. Detect: a test
  that edits dialogue after a duck and checks the note appears.
- [Peak cache grows with long libraries] → 8-bit pyramids (about 1.4 MB per hour); Settings → Storage lists and
  clears the cache. Detect: cache size is reported in the Storage pane and asserted in a test with a 2 h fixture.
- [Meter readings drift from the heard audio] → readings carry media time and are matched to `AudioPosition`.
  Detect: a loopback test plays a click every second and checks meter peaks within 1 UI frame of the heard click.
- [Scrub on cold cache sounds late] → prefetch the neighbouring windows in the scrub direction. Detect: a benchmark
  measures event-to-grain latency at p95, recorded in Results.
- [The new limiter changes existing exports] → for material that never exceeds −1 dBTP the limiter is a pass-through;
  golden audio exports compare old and new within 1 LSB on such fixtures. Detect: the golden test.
- [Volume keyframes in documents opened by an older app] → the older app ignores unknown keyframe properties when it
  reads them (current behaviour of the reader), so the file opens with static volume. Detect: an old-reader fixture
  test.

## Migration Plan

No document migration. `volume` is an added keyframe property value. Existing exports keep their behaviour except
that the true-peak limiter replaces the sample-peak limiter, covered by the golden test above.

## Open Questions

- Waves proceed in master-plan order (default); confirm priority against the open R1 and R2 performance gates.
- Analysis and models download on first use, with explicit consent above 50 MB. This change downloads nothing.
- DirectML stays off by default with a CPU fallback, pending a licence ruling (not used by this change).
- SVT-AV1 stays excluded pending a licence ruling (not used by this change).

## Results

To be filled during apply: analysis throughput (× real time), waveform draw time for 50 visible clips, scrub latency
p95 (cached and cold), meter CPU use, export loudness accuracy on fixtures, on the macOS and Windows reference
machines.

Measured on the Windows 11 x64 development machine (debug test builds unless stated). The macOS reference machine
and the built app (group 7) are not measured yet.

- Volume keyframes (group 1): a 100 % → 0 % linear fade over 1 s reads −30.00 dB at 0.5 s in the mixed output
  (`volume_keyframes_fade_in_db_and_preview_equals_export`); the 32-sample block gain stays within 0.1 dB of the dB
  curve. Mixing the same timeline in 1024-frame blocks (preview), 1600/1601-frame blocks (export per video frame)
  and odd 37/4093/512-frame blocks gives bit-identical samples.
- Import analysis (group 2): one pass over a 10 s stereo 48 kHz WAV takes 118 ms in a debug build (85× real time);
  three waveform range queries and one loudness query afterwards decode nothing (decoder opens: exactly 1). The
  peak file is 16 148 bytes for 10 s of stereo, so about 5.8 MB per hour, not the 1.4 MB D2 estimated: the 256-frame
  base level of two channels alone is 2.7 MB per hour and the upper levels add as much again. Still small next to
  the media; the Storage pane will list and clear it (background-tasks task 6.3).
- The analysis runs as a `derivative` task in the `cpu` class on the shared scheduler, keyed by the media content
  id; the peak cache Storage row is deferred to the background-tasks change.
