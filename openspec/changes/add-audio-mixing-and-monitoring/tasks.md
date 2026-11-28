# Tasks: add-audio-mixing-and-monitoring

## 1. Volume keyframes

- [x] 1.1 Add `volume` to the keyframe property list in `ove-core` with the 0–200 range check, and to `timeline.d.ts`
      and the projection printer and reader; verify a golden projection with volume keyframes round-trips byte for
      byte and an out-of-range value gives the expected diagnostic.
- [x] 1.2 Evaluate volume keyframes in the mixer in dB per 32-sample block, multiplied with static volume and fades;
      verify a unit test of a 100→0 linear fade over 1 s against the expected dB curve within 0.1 dB, and a test that
      preview and export mixes of the same timeline are sample-identical.
- [x] 1.3 Document volume keyframes in `docs/cli` and the agent guide (`ove docs agents`); verify the executed doc
      example applies cleanly with `ove source apply`.

## 2. Import analysis cache

- [x] 2.1 Implement the analysis pass: one decode producing the per-channel min/max peak pyramid, the loudness summary
      and the onset envelope, written to the bundle cache under the media key; verify a test that counts decoder opens
      (exactly one for import, three waveform queries and one loudness query).
- [x] 2.2 Run the pass as a background task after import (on the scheduler when available, else the import worker) and
      invalidate it when content changes; verify a changed-file test re-analyses and an unreadable-audio fixture
      reports an analysis error without blocking other media.
- [x] 2.3 Add the peak range query command (source range, bins per pixel); verify it picks the coarsest sufficient
      level and returns values equal to a brute-force reference on a fixture.

## 3. Waveforms and gain line

- [x] 3.1 Draw clip waveforms on the timeline canvas from the range query, mapped through trim, speed and reverse and
      scaled by gain; verify UI tests for a trimmed reversed clip and a gain change, and a draw-time benchmark for 50
      visible clips recorded in design.md Results.
- [x] 3.2 Add the gain line: drag changes static volume, Alt/Option-click adds a keyframe, point drag and delete; one
      revision per gesture; verify UI tests for each gesture and that undo restores the previous state.

## 4. Meters and scrub audio

- [x] 4.1 Add the meter tap (peak, RMS, true peak, momentary and short-term loudness per channel, tagged with media
      time) and the throttled event; verify a −6 dBFS sine reads −6.0 dB within 0.1 dB and a loopback click test
      aligns within one UI frame.
- [x] 4.2 Implement meter ballistics as a pure function and draw the meters; verify unit tests for fall rates, 1.5 s
      peak hold and the latched clip indicator, and measure meter CPU use (target under 1 %).
- [x] 4.3 Implement scrub grains from the shared mixer with the revision-keyed window cache and direction prefetch;
      verify latency p95 (cached ≤ 40 ms, cold ≤ 120 ms) recorded in Results, no discontinuity at grain boundaries
      larger than the signal step plus 1e-3, and that an edit changes the next grain.

## 5. Ducking

- [x] 5.1 Implement speech spans from transcript word timings and from the level gate with hysteresis; verify unit
      tests on a synthetic dialogue fixture for both sources.
- [x] 5.2 Implement keyframe generation (attack, release, hold, curve simplification to at most 2 keyframes per
      second, merge and replace modes) as one revision with dry run; verify the music level is −12 dB within 0.5 dB
      during speech and unchanged within 0.2 dB elsewhere, ramp timing within 10 ms, the no-speech and unknown-id
      refusals, and that undo restores the previous keyframes.
- [x] 5.3 Expose `ove audio duck`, MCP `duck_audio` and the Duck popover in the Audio panel; verify a CLI test with
      `--dry-run --json`, an MCP contract test, and a UI test of the popover; update `docs/cli`.

## 6. Loudness and audio-only export

- [x] 6.1 Move the BS.1770 core into `ove-render` and re-export it from `ove-analyze`; verify existing loudness tests
      pass unchanged and results still match FFmpeg's `ebur128` within 0.1 LU.
- [x] 6.2 Replace the sample-peak limiter with the true-peak look-ahead limiter; verify true peak ≤ −1.0 dBTP on a hot
      fixture and golden exports of quiet fixtures unchanged within 1 LSB.
- [x] 6.3 Add loudness presets to export (two-pass measure and gain) with before and after values in the result, plus
      `ove audio loudness --timeline`; verify a −30 LUFS fixture exports at −14.0 ± 0.5 LUFS and the silent-timeline
      case completes without gain.
- [x] 6.4 Add the audio kind to the export settings (m4a with priming edit list, WAV 24-bit, FLAC) and
      `ove media extract-audio` with MCP `extract_audio`; verify WAV samples match the mixer within 1 LSB, m4a
      duration equals the timeline within one AAC frame, the offline-media refusal leaves no file, and the no-audio
      refusal; update `docs/cli`.

## 7. Integration checks

- [ ] 7.1 In the built app on macOS and Windows: import a long interview, see waveforms appear, drag gain lines, add
      volume keyframes, scrub, watch the meters, duck music under dialogue, and undo each step; record timings in
      design.md Results.
- [ ] 7.2 In the built app on macOS and Windows: export at −14 LUFS and as audio-only WAV and m4a; verify loudness and
      true peak with `ove audio loudness` and FFmpeg `ebur128`, and record results.
- [ ] 7.3 Run cargo fmt, cargo clippy -D warnings, cargo test, cargo deny check and the UI tests on all CI targets.
