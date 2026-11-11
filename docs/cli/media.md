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
```

`<media>` is a media id, a file name or a path already in the project; the analysis commands also take any file.

- **add** probes every file first, then adds media and clips in one revision: video and images on Main, audio-only
  files on Audio (or `--track`), one after another from the end of the track (or from `--at`). A file already in the
  project is reused. Images get `--image-duration` (5 s).
- **remove** deletes the media and every clip that uses it. **relink** points media at a moved file, or finds every
  offline file by name under `--folder` (4 levels deep).
- **grab** decodes the frame at a time (the last frame past the end); **filmstrip** makes a contact sheet of `--count`
  frames (or one every `--every` seconds); **scenes** lists hard cuts with a score (default threshold 0.3).
- **levels** gives peak and RMS dBFS per bucket; **silence** lists ranges quieter than `--threshold` (−40 dB) for at
  least `--min` (0.5 s); **loudness** measures integrated loudness (LUFS, ITU-R BS.1770 / EBU R128 gating),
  loudness range, sample peak and true peak.

These commands need a build with media support (desktop bundles; `cargo build -p ove-cli --features media`).
