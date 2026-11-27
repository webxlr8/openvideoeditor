# Tasks: add-gpu-compositor-and-scopes

## 1. Numeric colour readout (CPU, ships first)

- [x] 1.1 Implement `ove-analyze::color_stats` with the full metric set, zones, CIELAB balance and skin-line deviation;
      verify unit tests on synthetic frames with known values (grey ramp, colour bars, warm and cool casts).
- [x] 1.2 Add `ove color inspect` with `--at`, `--item`, `--isolate`, `--reference`, `--json`, measuring through the
      compositor at the exact frame; verify byte-identical repeat output and the exit-code-1 cases in CLI tests.
- [x] 1.3 Add reference gap and `suggestedOps` built from `fit_match` with a strength cap; register MCP
      `inspect_color`; document both in `docs/cli/`; verify the docs examples run in the integration tests.
- [x] 1.4 Mock-agent loop test; verify a warm-shifted shot reaches mean CIEDE2000 < 2 against the reference within
      3 iterations.

## 2. Backend trait and parity harness

- [x] 2.1 Introduce the backend trait in `ove-render` and wrap the current CPU code as the CPU backend; verify all
      existing renderer tests and the export golden test pass unchanged.
- [ ] 2.2 Build the tolerance parity harness (charts plus 3 reference frames, max, mean and CIEDE2000 p99) and replace
      the Linux-only effect golden hashes; verify the harness runs on Linux, macOS and Windows CI.
- [ ] 2.3 Add WARP and lavapipe GPU jobs to CI and a nightly real-GPU job that records timings; verify a deliberately
      wrong shader fails the parity job and names the case.

## 3. GPU compositor core

- [x] 3.1 Layer draw (affine, crop, corner radius, opacity), all 16 blend modes, grade lattice as a 3D texture with
      tetrahedral lookup, and LUT stages; verify parity for every blend mode and for the 9 LUT presets.
- [x] 3.2 CPU fallback per layer for effects without a GPU version (readback, CPU effect, upload); verify a project
      using every effect renders on the GPU backend with parity.
- [x] 3.3 Startup self-test, backend selection, Settings toggle, `ove render --backend`, diagnostics overlay reason,
      device-loss fallback; verify by tests that force self-test failure and simulate device loss.

## 4. Effects, transitions and alpha

- [x] 4.1 Change the per-pixel effect contract to RGBA; verify every existing effect keeps input alpha (test on a
      semi-transparent layer).
- [x] 4.2 Keyframable effect parameters using the shared keyframe evaluator, with an apply diagnostic for unknown
      parameters; verify the animated blur scenario and the refusal scenario.
- [ ] 4.3 Port separable blurs, then the 24 transitions, then the remaining per-pixel effects, then distortion effects
      to WGSL with the shared integer noise hash; verify parity for each one as it lands.
      Status (open): native on the GPU: zoom-blur, tilt-shift, aurora and the wipe, radial-wipe, ink-bleed and
      mask-glitch masks (every transition's motion and opacity runs in the native draw). On the per-layer CPU
      fallback: the other 38 effects (also the glitch, chromatic-aberration and light-leak post effects of
      transitions). Ported in order of measured cost (`ove-gpu/tests/costs.rs`); watercolor failed parity and stays on
      the fallback.
- [ ] 4.4 Raise timeline preview to 1080p when the GPU backend is active; verify 0 dropped frames over 60 s for the
      four-layer project on the reference machines and record numbers in Results.

## 5. Chroma key

- [ ] 5.1 Implement `chroma-key` (CbCr distance scaled by luma, spill suppression, choke, matte levels, edge blur,
      matte view) on CPU and GPU; verify against the CC0 green and blue test plates with ground-truth mattes (SAD target,
      edge spill < 0.02, interior CIEDE2000 < 1).
      Status (open): built and verified on synthetic green and blue plates with exact ground-truth mattes (numbers
      in design.md Results); the CC0 plates are not in the repository yet.
- [x] 5.2 Viewer sampler for point and rectangle sampling before grading, one revision per sample, outside-layer
      feedback; add `look --view matte`; verify UI tests and the agent matte-view scenario.
- [x] 5.3 Document the effect and its parameters in `docs/spec/presets.md` and `ove docs`; verify the docs list matches
      the effect registry in a test.

## 6. Visual scopes

- [ ] 6.1 Implement waveform, parade and vectorscope binning on CPU and as a GPU compute pass, reading the playback
      frame; verify the colour-bars scenario and cost ≤ 2 ms GPU or ≤ 6 ms CPU per 540p frame.
- [ ] 6.2 Stream binned u8 arrays to the UI at up to 15 Hz and draw them in the colour panel; verify the paused-update
      scenario in a UI test.
- [ ] 6.3 Add `ove look --scope`; verify the vectorscope image and unknown-scope scenarios in CLI tests.

## 7. Integration checks

- [ ] 7.1 Built app on macOS and Windows: play a 4-layer 1080p project with grade and effects, key a green-screen clip
      with the sampler, read scopes; verify no dropped frames and record numbers in design.md Results.
- [ ] 7.2 Export 4K on both reference machines with each backend; verify GPU export ≥ 1.5× realtime and the CPU and GPU
      files match within the parity tolerance.
