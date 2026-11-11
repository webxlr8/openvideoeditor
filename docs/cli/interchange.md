# `ove timeline export` / `ove timeline import`

Exchange a project's timeline with other editors.

| Format | Extension | Opens in |
|---|---|---|
| FCPXML 1.11 | `.fcpxml` | Final Cut Pro, DaVinci Resolve |
| Final Cut Pro 7 XML (xmeml v5) | `.xml` | Premiere Pro, DaVinci Resolve, Final Cut Pro 7 |
| CMX3600 EDL | `.edl` | Most editors and finishing tools (one video track, one audio track) |
| OpenTimelineIO | `.otio` | OpenTimelineIO tools and adapters |

```sh
ove timeline export <project> <file> [--format fcpxml|xml|edl|otio] [--fps <n>] [--json]
ove timeline import <project> <file> [--format fcpxml|xml|edl|otio] [--fps <n>] [--json]
```

The format comes from `--format`, else from the file's extension. `--fps` (default 30; 29.97 and 23.976 are allowed)
sets the frame rate of an exported file and of an imported EDL (the other formats carry their own rate).

**Export** writes the visible video, image, audio and text items. Overlapping items on one track are spread over
extra lanes. Trims and constant speed changes are kept; titles become FCPXML Basic Title / FCP7 Text generators
(EDLs carry only the main video track and the first audio track, with media paths in `* SOURCE FILE:` comments).
Captions, code/shader items, adjustment layers, effects and keyframes are not exported.

**Import** adds the file's tracks and clips to the project as one revision: video tracks above the existing tracks,
audio tracks at the bottom, all named "Imported …". Media already in the project is reused by path; other files are
added and probed when the project is next opened in the app. EDL events without a path are matched by clip name
against the project's media. Clips whose media is not found become pending placeholders (reported under `missing`).
Undo removes the whole import.

Examples (exit codes as documented):

```sh
ove timeline export Demo cut.otio # exit 0
ove timeline export Demo cut.fcpxml --fps 29.97 --json # exit 0
ove timeline export Demo cut.xml # exit 0
ove timeline export Demo cut.edl # exit 0
ove timeline export Demo cut.mov # exit 1
ove timeline import Demo cut.otio --json # exit 0
ove timeline import Demo missing.edl # exit 1
```

JSON output: export `{"path", "format", "bytes"}`; import `{"rev", "tracks", "clips", "mediaAdded", "missing"}`.
Errors: `format.unknown`, `file.unreadable`, `file.unwritable`, `import.invalid` (the file does not parse),
`import.empty` (no clips), `project.locked`.
