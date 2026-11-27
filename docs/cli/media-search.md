# Media search

Find moments in a project's footage by what they show and what is said, and organise media into folders, tags and
subclips. Everything runs on this computer and reads the project bundle, so the commands work on a closed project
without the app. The same features are agent tools in the `ove-tools` registry: `search_media`, `media_transcript`,
`inspect_media` and `organize_media` (see [tools.md](tools.md)).

## What is searched

| Section | Finds | Needs |
|---|---|---|
| Moments | Shots whose picture matches the words, or that look like a given frame ("more like this") | The visual search model, and the media indexed |
| Spoken | Transcribed speech; a quoted phrase matches those words in that order | Transcripts (`ove transcribe`) |
| Timelines | Clips and titles on the timeline whose name or text matches | — |
| Files | Media names, file names, folders and tags | — |

Case and accents are ignored. Each Moments and Spoken hit has a `clip` line, for example
`<clip start={0} end={4} in={12.4} src="harbour" />`. Paste it into a track of `timeline.tsx`, set `start` and `end`
where it goes (keep `end − start`), and apply: the clip plays the hit's range.

A Moments hit gives the shot range, the best frame, a suggested window of up to 4 seconds around it inside the shot,
and a confidence between 0 and 1. Results below 0.1 are left out. A Spoken hit runs from the first matched word's
start to the last one's end, plus 150 ms on each side.

## The visual search model

The model (SigLIP 2, about 0.63 GB) is downloaded only after the person agrees: in Settings > Storage, or with
`ove models install siglip2-base --yes`. No search command and no agent tool downloads it. Without it, `index`
in the result says `notInstalled` with the download size, and Moments stays empty. The other sections still work.

With the model installed, the app indexes each video and image in the background (a low-priority task): it splits
videos into shots at cuts, samples the sharpest frame near the middle of each shot and every 8 seconds, skips black
and flat frames, and stores the index in the bundle at `cache/embeddings/<contentId>.safetensors`. A moved or
relinked file keeps its index. A damaged index file is deleted and rebuilt.

## Commands

The examples use a project named `Lib` with two videos, `harbour` and `market`. The exit code is shown after each
command.

```sh
ove media search Lib '"the price is"' --scope spoken --json   # exit 0
ove media search Lib market --scope files --json              # exit 0
ove media search Lib --like harbour@4 --scope moments --json  # exit 0
ove media search Lib --json                                   # exit 1
ove media transcript Lib harbour --from 2 --to 4 --json       # exit 0
ove media transcript Lib market --json                        # exit 1
ove media organize Lib '[{"op":"move","media":["harbour","market"],"folder":"Selects"}]' --json  # exit 0
ove media organize Lib '[{"op":"saveSubclip","media":"harbour","in":3,"out":5}]' --json         # exit 0
```

- `ove media search <project> [query] [--scope all|moments|spoken|timelines|files] [--media M] [--limit N]
  [--like MEDIA@SECONDS]`: `--like` uses a frame as the query.
- `ove media transcript <project> <media> [--from S] [--to S]`: the transcript in source seconds.
- `ove media overview <project> <media> [--out sheet.png] [--tile-width 240]`: one sheet of at most 36 tiles with time
  labels, one per shot (needs a build with media support).
- `ove media organize <project> <ops>`: a JSON array of steps, or `@file.json`. All steps are one revision, so one
  undo restores them. Steps: `move` (`media`, `folder`), `rename` (`media`, `name`), `tag` (`media`, `add`,
  `remove`), `createFolder` (`path`), `renameFolder` (`from`, `to`), `deleteFolder` (`path`; its media move to the
  parent), `saveSubclip` (`media`, `in`, `out`, `name`), `removeSubclip` (`media`, `id`) and `smartFolder` (`path`,
  `query`, `scope`).

Organisation is project data (`folder`, `tags`, `subclips` and a display `name` on media items; `folders` on the
project). A folder rename made by the person and tags added by an agent at the same time are both kept. Folder paths
appear in the media table of `timeline.tsx`.

## Index health

`search_media` returns `index`:

| Field | Meaning |
|---|---|
| `visual` | `notInstalled`, `indexing` (some files have no index yet) or `ready` |
| `indexed`, `total` | Indexed videos and images, of all |
| `downloadBytes`, `message` | When the model is not installed: its size and how to install it |
| `transcribed`, `untranscribed` | Media with and without a transcript (spoken search covers only transcribed media) |
