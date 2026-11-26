# Tasks: improve-interchange-and-packaging

## 1. Interchange model and OTIO

- [ ] 1.1 Extend the interchange model with transform, crop, opacity, volume in dB, fades, keyframes, enabled and source
      timecode; add source timecode to the media probe if `add-audio-intelligence` has not; verify unit tests for the
      document-to-model conversion of each property.
- [ ] 1.2 Write the full item under `metadata.ove` in OTIO and prefer it on import, with native fallback and report;
      verify the round-trip and extra-data-stripped scenarios.

## 2. FCPXML and FCP7 XML

- [ ] 2.1 Add target and version to the FCPXML writer with per-target conversion tables (position, scale, rotation,
      crop, compound references for one-sided use, retime maps, keyframe offsets, volume dB); verify golden XML tests per
      target and version and the unsupported-version refusal.
- [ ] 2.2 Add Basic Motion, Crop, Opacity and Audio Levels to the FCP7 XML writer; verify golden XML tests.
- [ ] 2.3 Measure the conversion factors by round trips in DaVinci Resolve 19/20 and Final Cut Pro 11 and record them in
      `docs/cli/interchange.md` with a manual checklist per editor release; verify the Resolve framing scenario with
      SSIM ≥ 0.95.
- [ ] 2.4 Read the extended properties back in the FCPXML, FCP7 XML and OTIO importers; verify round-trip tests within
      the stated tolerances for each format.

## 3. Fidelity report

- [ ] 3.1 Make the report aware of format, target and version, with counts and example names; verify the
      report-lists-effects and nothing-lost scenarios.
- [ ] 3.2 Add `--target` and `--version` to `ove timeline export`, `target` and `version` to MCP `export_timeline` /
      `import_timeline`, and the pickers plus report to the export dialog; verify CLI, MCP contract and UI tests; update
      `docs/cli/interchange.md` with executed examples.

## 4. Packaging

- [ ] 4.1 Implement the packaging job (space check, `.partial` staging, bundle copy, content deduplication with full-hash
      confirmation, one relink batch in the copy, rename); verify the opens-elsewhere (on a second user account),
      source-unchanged, interrupted, same-file-twice and not-enough-space scenarios.
- [ ] 4.2 Implement trim to used with handles through the export pipeline, proxies and zip options; verify the trimmed
      package scenario.
- [ ] 4.3 Implement the package report, missing-media handling and strict mode; verify the missing-file and strict
      scenarios.
- [ ] 4.4 Add `ove project package`, MCP `package_project`, File → Package Project with progress in the Exports panel;
      verify CLI, MCP and UI tests including the destination-exists refusal; document in `docs/cli/project.md`.

## 5. Integration checks

- [ ] 5.1 In the built app on macOS and Windows: export the sample project for Resolve and open it in DaVinci Resolve;
      on macOS also export for Final Cut Pro and open it there; verify framing, levels and fades by eye and by SSIM.
- [ ] 5.2 On both OSes: package a project with external media, copy it to the other OS, open it; verify no offline
      media and record packaging speed (MB/s) in design.md Results.
