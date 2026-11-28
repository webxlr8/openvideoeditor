# Tasks: add-color-management-and-subject-mattes

## 1. Colour space model and transforms

- [ ] 1.1 Add `project.color` (working, output, peak brightness), media `colorSpace`, effect `color-transform` and LUT
      `space` to the schema, projection and `timeline.d.ts`; verify projection round-trip tests and the unknown-name
      refusal.
- [ ] 1.2 Implement `colorspace.rs`: transfer curves (sRGB, BT.1886, PQ, HLG, Apple Log, S-Log3, LogC3, LogC4, V-Log,
      Log3G10, ACEScct, DWG Intermediate) and primaries matrices; verify each curve against its published table within
      1e-4 and the Rec.709 round trip within 0.5/1023.
- [ ] 1.3 Input space detection from file tags and override as one revision; verify detection on HLG, PQ and Rec.709
      fixtures and the untagged-log override scenario.
- [ ] 1.4 Document colour fields and the transform effect in `docs/spec/project-schema-v1.md` and `ove docs`; verify
      the typings drift check passes.

## 2. Pipeline

- [ ] 2.1 Stop forcing SDR at decode; carry source transfer and primaries to the renderer; verify old-project golden
      frames are unchanged with default settings.
- [ ] 2.2 Grade lattice in the working encoding (65³ half float where needed), linear-light compositing, output
      transform with tone map, on GPU (rgba16f) and CPU (f32 oracle); verify parity within the compositor tolerance and
      lattice error below 0.1%.
- [ ] 2.3 LUT `space` handling; verify the log-LUT-in-ACEScct scenario against a reference render.

## 3. HDR output and preview

- [ ] 3.1 HDR export frames (10-bit, PQ and HLG, BT.2020) with mastering display and content light metadata, SDR
      graphics at 203 nits, using encoders from extend-export-formats-and-queue; verify ffprobe tags, the 0.5807 PQ
      value test, and the no-encoder refusal.
- [ ] 3.2 HDR preview on capable displays with tone-mapped SDR fallback and a diagnostics overlay message; verify on
      a macOS XDR display and a Windows HDR10 monitor, and the SDR-display scenario.
- [ ] 3.3 Round-trip an iPhone HLG clip through edit and HLG export; verify by visual check and ffprobe, recorded in
      Results.

## 4. Subject mattes

- [ ] 4.1 Pick the model from the candidates by test-set IoU, speed and weight licence; record the licence text and
      SHA in Results; verify the licence is Apache-2.0 or MIT for code and weights.
- [ ] 4.2 Implement `matte.rs` in the inference worker: 512 px inference, temporal smoothing, lossless sidecar keyed by
      `contentId`, run as a background job with progress and cancel; verify IoU ≥ 0.9, flicker < 0.01, cache reuse and
      cancel within one second.
- [ ] 4.3 Download with consent above 50 MB through add-model-runtime; verify the consent-declined scenario leaves no
      files and shows the unavailable state.
- [ ] 4.4 Guided-filter upsampling in the renderer and `look --view matte`; verify golden edges and the agent matte
      scenario.

## 5. Features driven by mattes

- [ ] 5.1 Effect `region: subject|background`; verify the background-blur golden frame and the missing-matte fallback,
      warning and export refusal.
- [ ] 5.2 Captions `behind-subject` and footage-filled text with the subject as source; verify golden frames.
- [ ] 5.3 `ove color inspect --region subject`; verify the skin readout scenario and the no-matte refusal.
- [ ] 5.4 Mark add-effects-and-color task 1.4 as covered by this change; verify the task list references this change.

## 6. Integration checks

- [ ] 6.1 Built app on macOS and Windows: open an S-Log3 clip and an HLG clip, set input spaces, grade in ACEScct,
      export Rec.709 and HLG, compute a matte, blur the background, place a caption behind the person; verify outputs by
      ffprobe and golden frames, and record matte speed (target ≥ 60 fps at 512 px on M1) and installer size (unchanged)
      in design.md Results.
