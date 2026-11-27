# Tasks: add-audio-intelligence

## 1. Source timecode

- [x] 1.1 Read the timecode tag in probe and add our own Broadcast WAV and iXML chunk reader; store `sourceTimecode`
      on media entries; verify fixtures for a 23.976 drop-frame video, a 25 fps video, a Broadcast WAV and a file
      without timecode, and that a video and a WAV with shared timecode align to the exact sample.
- [x] 1.2 Show timecode in the media details and in `ove media probe --json`; verify a CLI golden test and update
      `docs/cli`.

## 2. Audio sync

- [x] 2.1 Implement the coarse FFT search with overlap normalisation over the cached onset envelope, the minimum
      overlap and the peak-ratio ambiguity check; verify a synthetic fixture (speech-like bursts plus pink noise,
      12.345 s delay, low-passed, 10 dB SNR) finds the offset within 10 ms and a looped-music fixture is refused.
- [x] 2.2 Implement the fine phase-transform refinement with parabolic interpolation; verify offset error ≤ 1 ms on
      the synthetic fixture.
- [x] 2.3 Implement drift windows with the Theil–Sen fit, the 20–500 ppm acceptance range and residual reporting, and
      apply the correction to the audio-only target's speed; verify a +50 ppm resampled two-hour fixture gives drift
      within 3 ppm, head error ≤ 1 ms and tail error ≤ 2 ms, and a 2,000 ppm case is reported and not applied.
- [x] 2.4 Implement placement (confidence order, anchor chaining, group shift right, overlap refusal) as one apply
      batch with dry run, plus timecode mode; verify unit tests for each rule, that one revision is created, and that
      the unrelated-audio case exits non-zero without editing.
- [x] 2.5 Expose `ove audio sync`, MCP `sync_items` and the timeline "Synchronise selection" command with a result
      summary; verify a CLI `--dry-run --json` test, an MCP contract test, a UI test, the run time of a 2 h reference
      against 10 targets with cached envelopes (target under 10 s) in design.md Results, and update `docs/cli`.

## 3. Beats

- [ ] 3.1 Export the beat model to ONNX with our own script, record its weight licence, and register it with the
      model runtime; verify the parity gate (logits within 1e-3, beats identical within one frame against the
      reference implementation on fixtures).
- [ ] 3.2 Implement chunked inference, peak picking, downbeats, bars, tempo curve and the cache; verify a 120 BPM
      click track (500 ms ± 20 ms, 120 ± 0.5 BPM), a three-four fixture, and the "model not installed" status without
      download.
- [ ] 3.3 Map beats to timeline seconds through trim, speed and reverse; expose `ove audio beats` and MCP
      `detect_beats`; verify property tests under speed and reverse, a CLI golden test and an MCP contract test;
      update `docs/cli`.
- [ ] 3.4 Feed beat targets to the snap engine, with downbeats ranked first, and draw beat marks behind the "Snap to
      beats" toggle; verify UI tests that an edge lands on a downbeat and that no marks or snapping happen when the
      toggle is off.

## 4. Denoise

- [ ] 4.1 Add the `denoise` sound field (0–100) to the schema, projection and typings; verify a golden round trip and
      an out-of-range diagnostic.
- [ ] 4.2 Integrate the speech enhancement library and model through the model runtime, record the weight licence,
      and render the delay-compensated cleaned FLAC as a background task; verify the speech-in-noise fixture (noise
      parts ≥ 20 dB quieter, speech within 1 dB) and that the real-time factor is under 0.2 on both reference
      machines, recorded in Results.
- [ ] 4.3 Blend original and cleaned sound in the mixer by the amount, with original-only playback and a progress chip
      until ready; verify a test that amount 50 equals the average of both and that the cross-correlation peak of
      clean and original is at lag 0 ± 1 sample.
- [ ] 4.4 Add the export rule (render missing copies, refuse on failure, `--allow-unprocessed`); expose
      `ove audio denoise`, MCP `denoise_audio` and the Inspector slider; verify export tests for the missing, failed
      and allowed cases, an MCP contract test and a UI test; update `docs/cli`.

## 5. Integration checks

- [ ] 5.1 In the built app on macOS and Windows: sync a camera and a recorder with drift, check the result summary,
      play back across the whole length and confirm no audible echo at start and end; record offsets in Results.
- [ ] 5.2 In the built app on macOS and Windows: detect beats on a music clip, cut three clips to downbeats with
      "Snap to beats", denoise a noisy interview at 70, export, and confirm the exported sound is cleaned.
- [ ] 5.3 Run cargo fmt, cargo clippy -D warnings, cargo test, cargo deny check and the UI tests on all CI targets.
