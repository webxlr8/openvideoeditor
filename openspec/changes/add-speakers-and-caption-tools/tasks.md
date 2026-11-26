# Tasks: add-speakers-and-caption-tools

## 1. Speaker identity

- [ ] 1.1 Review the licences of the candidate speaker-model weights and pin one model with its SHA-256 and size;
      verify the licence review is recorded in design.md and the model downloads through the shared downloader.
- [ ] 1.2 Implement window embeddings over speech cells, per-file clustering and per-word labels, cached by content
      identity; verify DER at most 15% on the 2-speaker 10-minute fixture and record the speed per OS in Results.
- [ ] 1.3 Implement cross-file matching with optimal assignment and centroid updates; verify the same person in 3
      files gets 1 speaker and two analysis orders give identical labels.
- [ ] 1.4 Add the project speaker list and per-media speaker corrections to the document with migration, projection
      header printing and apply; verify round-trip, migration and merge tests and the agent-rename scenario.
- [ ] 1.5 Add speaker runs to the timeline transcript; CLI `ove speakers` and MCP `identify_speakers`, documented in
      `docs/cli/`; verify CLI and MCP contract tests including the unknown-speaker refusal.
- [ ] 1.6 Audio panel Speakers section (rename, recolour, merge, split, threshold) and speaker tint on waveforms;
      verify UI tests for each action and that each action is one revision.

## 2. Caption building and subtitle files

- [ ] 2.1 Add segmentation settings (words, characters, lines, gap, case) with the renderer-based fit check and the gap
      and overlap pass; verify golden tests at 9:16 and 16:9 and the impossible-limit warning.
- [ ] 2.2 Expose the settings in the Captions panel, `ove audio captions` flags from docs/spec/cli.md and
      `caption_from_transcript` options; verify CLI and MCP tests and the docs examples.
- [ ] 2.3 Implement the SRT and WebVTT parser with encoding detection, and import as one revision; verify a fixture
      corpus (CRLF, BOM, Windows-1252, styling tags, hours above 99, broken time line).
- [ ] 2.4 Implement SRT and WebVTT export with optional WebVTT word timestamps; CLI `ove captions import|export`, MCP
      `import_subtitles` and `export_subtitles`, panel buttons; verify the round-trip and no-caption-track scenarios.

## 3. Styles and translation

- [ ] 3.1 Render gallery previews with `ove-render`, cached per preset and aspect, and remove CSS previews; verify a
      golden test that preview frames equal export frames.
- [ ] 3.2 Add optional per-word styles (colour, scale, emphasis) to caption word timings, `timeline.d.ts`, the
      renderer and the Inspector; verify schema round-trip, the out-of-range warning and golden frames.
- [ ] 3.3 Implement `translate_captions` through the connected agent with id validation, one retry and character-share
      word timing; CLI `ove captions translate`; verify mock-agent tests for success, wrong ids and no agent.

## 4. Integration checks

- [ ] 4.1 In the built app on macOS and on Windows: analyse a 3-file podcast, check speakers match across files, rename
      and recolour a speaker, and see the waveform tint; verify each action appears in History and undoes.
- [ ] 4.2 In the built app on both OSes: build captions with a 2-line limit, import an SRT, translate a track through
      the assistant, export SRT and WebVTT; verify the exported files open in a common video player with correct text
      and timing.
- [ ] 4.3 An external agent over MCP renames a speaker through `timeline.tsx`, styles a keyword and exports subtitles;
      verify the changes appear live in the open app.
