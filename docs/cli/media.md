# `ove media` and `ove audio`

```sh
ove media list <project> [--all]
ove media add <project> <paths...> [--track <name|id>] [--at <s>] [--no-timeline] [--image-duration <s>]
ove media remove <project> <media...>
ove media relink <project> <media> <path>
ove media relink <project> [<media>] --folder <dir>
ove media probe <project> <media|path>
ove media grab <project> <media|path> [--at <s>] [--width <px>] --out <file.png>
ove media filmstrip <project> <media|path> --out <file.png> [--count <n>] [--every <s>] [--from <s>] [--to <s>] [--columns <n>] [--tile-width <px>]
ove media scenes <project> <media|path> [--threshold <0-1>]
ove audio levels <project> <media|path> [--points <n>]
ove audio waveform <project> <media|path> --out <file.png> [--width <px>] [--height <px>]
ove audio silence <project> <media|path> [--threshold <dB>] [--min <s>]
ove audio loudness <project> <media|path>
ove audio loudness <project> --timeline
ove media extract-audio <project> <media> [--format wav|flac]
ove audio duck <project> --music <items|tracks> --under <items|tracks> [--depth <dB>] [--attack <s>] [--release <s>] [--hold <s>] [--source transcript|level|auto] [--merge|--replace] [--dry-run]
ove audio sync <project> --ref <item> --targets <items|tracks> [--mode auto|timecode|audio] [--window <s>] [--min-confidence <0-1>] [--drift auto|off] [--dry-run]
```

`<media>` is a media id, a file name or a path already in the project; the analysis commands also take any file.

- **add** probes every file first, then adds media and clips in one revision: video and images on Main, audio-only
  files on Audio (or `--track`), one after another from the end of the track (or from `--at`). A file already in the
  project is reused. Images get `--image-duration` (5 s).
- **probe** prints container, streams, duration and frame rate. When the file carries a start timecode (a QuickTime
  timecode track, MXF, Broadcast WAV `time_reference` or iXML), it also prints `sourceTimecode`: `frames` since
  midnight, the exact rate as `rateNum`/`rateDen`, `dropFrame`, and for Broadcast WAV the exact start in `samples`
  at `sampleRate`. Files without timecode print no `sourceTimecode`. `add` keeps the timecode on the media entry
  (`data.sourceTimecode`), and `ove audio sync --mode timecode` uses it.
- **remove** deletes the media and every clip that uses it. **relink** points media at a moved file, or finds every
  offline file by name under `--folder` (4 levels deep).
- **grab** decodes the frame at a time (the last frame past the end); **filmstrip** makes a contact sheet of `--count`
  frames (or one every `--every` seconds); **scenes** lists hard cuts with a score (default threshold 0.3).
- **levels** gives peak and RMS dBFS per bucket; **silence** lists ranges quieter than `--threshold` (−40 dB) for at
  least `--min` (0.5 s); **loudness** measures integrated loudness (LUFS, ITU-R BS.1770 / EBU R128 gating),
  loudness range, sample peak and true peak.
- **loudness --timeline** measures the timeline mix as it would export (true-peak limiter on, no loudness target).
- **extract-audio** writes a media file's sound into the bundle's `media/` folder as 24-bit WAV (default) or FLAC
  and adds it to the library as a new audio media entry, in one revision (undo removes the entry). Refused with
  `media.noAudio` when the file has no audio stream and `media.offline` when it is offline. MCP: `extract_audio`.
- **duck** lowers music under dialogue. It writes volume keyframes on the music items (`--music`: item ids or track
  ids or names, comma-separated) as one revision, so the result shows on the clips' gain lines, prints in
  `timeline.tsx` and undoes in one step. The level ramps down over `--attack` (0.2 s) before each line of speech,
  stays `--depth` dB lower (−12) during it and comes back over `--release` (0.4 s); lines closer than `--hold`
  (0.5 s) count as one. Speech comes from the dialogue items' cached transcript (made by auto captions) when there
  is one, else from their level (`--source level` forces the level, `--source transcript` refuses when there is no
  transcript). `--replace` (the default) rewrites volume keyframes only inside the affected range, so ducking again
  with another depth replaces the earlier duck; `--merge` multiplies with existing volume keyframes. `--dry-run`
  prints the planned keyframes per music item and changes nothing. Refusals (exit 1): `item.notFound` names an
  unknown item or track, `duck.noSpeech` when the dialogue has no speech, `duck.noTranscript`, `duck.noSound` for an
  item without sound.

  ```json
  {"source": "level", "speech": [[2.0, 4.0], [6.0, 7.5]], "depth": -12, "dryRun": true, "rev": 1762000000000,
   "items": [{"id": "i_music", "from": 1.8, "to": 7.9, "keyframes": [[1.8, 100.0], [2.0, 25.12], [4.0, 25.12], …]}]}
  ```

  Keyframe pairs are item-relative seconds and percent. MCP: `duck_audio` with the same arguments (`music`, `under`,
  `depth`, `attack`, `release`, `hold`, `source`, `mode`, `dryRun`). In the app: Audio panel → Duck under dialogue….
- **sync** lines up recordings of the same moment (a camera and a sound recorder, several cameras) against the
  reference item `--ref`, as one revision. `--mode auto` (the default) uses source timecode when both media carry
  it (see **probe**), else the sound; `timecode` and `audio` force one method. Sound sync correlates the cached onset
  envelopes over every offset (or within `--window` seconds), refines the peak to below 1 ms on the raw audio, and
  needs at least 3 s of overlap. A match below `--min-confidence` (0.2), or one whose best peak is not clearly above
  the next one (repeated music), is not placed. For overlaps longer than 150 s it measures clock drift in windows
  every 30 s with a robust line fit: 20–500 ppm is corrected by changing the speed of an audio-only target (video
  items keep their speed); other values are reported, not applied (`--drift off` skips the measurement). The most
  confident target is placed first, and placed targets anchor targets that do not overlap the reference. Placement
  keeps sub-frame precision. If an item would start before 0, every synced item (the reference too) moves right by
  the same amount; a move that would overlap another item on the same track is refused and names that item.
  `--dry-run` prints the plan and changes nothing. Each target reports `status` (placed, unmatched, refused),
  `method`, `offsetSeconds` (the reference media time at which the target media starts), `confidence`,
  `driftPpm`, `driftApplied`, `residualMs`, the new `start` and `end`, `movedBy` and the `reason` when it is
  not placed. When no target is placed the command exits 1 with `sync.unmatched` and the same details. MCP:
  `sync_items`. In the app: select the clips, right-click the reference clip, then Synchronise selection; a summary
  shows what moved.

  ```json
  {"reference": "cam", "placed": 1, "groupShift": 0.0, "dryRun": true, "rev": 1762000000000,
   "targets": [{"item": "rec", "status": "placed", "method": "audio", "anchor": "cam", "offsetSeconds": 7.250012,
     "confidence": 0.61, "start": 7.250012, "end": 27.250012, "movedBy": -42.749988, "speed": 1.0}]}
  ```

These commands need a build with media support (desktop bundles; `cargo build -p ove-cli --features media`).
