# `ove transcript`

The words said on the timeline, read from the transcripts cached in the project (the same transcripts captions
use). Agents call the same tool as `get_transcript`; the app, the command line, MCP and the assistant get the same
words for the same project revision.

```sh
ove transcript <project> [--clips <id,id>] [--segments] [--offset N] [--limit N] [--json]
ove transcript search <project> <phrase> [--json]
ove transcript fillers <project> [--json]
ove transcript transcribe <project> <media> [--model M] [--language L] [--verbatim] [--force]
```

Every clip with sound is included, on every track, unless `--clips` names some. Each word is mapped through its
clip's trim, speed and direction into timeline time. A media file without a cached transcript is listed under
`untranscribed`; nothing is recognised by this command.

## Word ids

A word's id is `<mediaId>@<sourceMs>`: the media it comes from and its start in that media, in milliseconds. The id
does not change when the timeline changes (cuts, moves, trims, speed changes elsewhere), so an agent can read the
transcript once and name several words in later edits. When a recognition window overlap gives the same word twice
(same media, same start, same text), it is listed once.

## JSON

`--json` prints:

```json
{
  "rev": 12,
  "words": [
    {"id": "m_talk@2500", "text": "Hello", "start": 10.25, "end": 10.5, "clipId": "v1", "mediaId": "m_talk",
     "sourceStart": 2.5, "sourceEnd": 3.0, "filler": false,
     "contextFiller": false}
  ],
  "totalWords": 1,
  "offset": 0,
  "nextOffset": null,
  "untranscribed": []
}
```

- `rev`: the project revision the words were read at.
- `start`, `end`: timeline seconds. `sourceStart`, `sourceEnd`: seconds in the media file.
- `filler`: a hesitation sound in the transcript's language ("um", "uh", "erm" and so on). Words that are fillers
  only in some contexts ("like", "so") are not flagged.
- Pages hold at most 10,000 words: `nextOffset` is the `--offset` of the next page, or `null` on the last page.

With `--segments`, `words` is replaced by `segments`: sentences, each `{"text", "start", "end", "clipId",
"wordIds"}`. A sentence ends after `.`, `?` or `!`, at a clip change, or at a gap of more than 1 second.

Exit codes: 0 done, 1 refused (`project.notFound`, `transcript.noClips` when `--clips` names no clip with sound).

## Examples

In a library with the project "Interview" (one clip, transcribed):

```sh
ove transcript Interview --json             # exit 0
ove transcript Interview --segments --json  # exit 0
ove transcript Interview --limit 2 --json   # exit 0
ove transcript Interview --clips nope       # exit 1
ove transcript Nobody                       # exit 1
ove transcript search Interview Hello --json                         # exit 0
ove transcript fillers Interview --json                              # exit 0
ove edit remove-words Interview --words m_talk@4000 --dry-run --json # exit 0
ove edit remove-words Interview --words m_talk@99999                 # exit 1
ove transcript transcribe Interview talk.wav --model tiny            # exit 1
ove edit remove-words Interview --match next --json                  # exit 0
```

The fourth example refuses because no clip has that id; the ninth because no word has that id
(`transcript.wordMissing`, naming it); the tenth because the speech model is a 78 MB download that needs your
consent first (`ove models install whisper-tiny --yes`), or, in a build without speech recognition,
`speech.unavailable`.

## Search

```sh
ove transcript search <project> <phrase> [--json]
```

Finds a phrase in every transcribed media file of the project, not only what is on the timeline. Case, accents and
punctuation are ignored ("cafe" finds "Café,"), and a phrase may run across sentences. Each hit gives `mediaId`,
`text`, `sourceStart` and `sourceEnd` (the first word's start and the last word's end, in seconds of the media),
`wordIds`, and `uses`: every clip that plays that range, with its timeline `start` and `end`. `untranscribed` counts
the media files with sound that have no transcript yet.

## Transcribe

```sh
ove transcript transcribe <project> <media> [--model tiny|base|small] [--language xx] [--verbatim] [--force] [--json]
```

Recognises the speech of a media file (an id or a file name) on this computer and caches the words in the project
under the file's content identity, the model, the language and the verbatim flag; a renamed or moved file keeps its
transcript. A cached transcript is returned at once unless `--force`. `--verbatim` keeps hesitations ("um", "uh");
it is cached apart from the normal transcript. The speech models are downloads over 50 MB, so they need your consent
first; agents get `model.consent` with the size. Builds of `ove` without speech recognition (`--features speech`)
refuse with `speech.unavailable`. MCP: `transcribe_media`.

## Fillers

```sh
ove transcript fillers <project> [--json]
```

Lists filler candidates for review; nothing is cut. Hesitation words (per language: "um", "uh", "erm" in English)
are `selected`; words that are fillers only in some contexts ("like", "so", "you know") are flagged with `context`
and never selected; after dead-air detection, voiced sounds of 0.2–1.2 s that no recognised word covers are listed
as `kind: "voiced"`. Cut the ones you confirm with `remove-words`. MCP: `find_fillers`.

## Remove words

```sh
ove edit remove-words <project> [--words <id,id>] [--word-range FROM..TO] [--match <phrase>]
    [--pause tight|balanced|loose] [--rev N] [--dry-run] [--json]
```

Cuts the selected words from the timeline as one revision and closes the gaps. Words are given by id, by an
inclusive range of ids in timeline order, or by an exact phrase (every occurrence). Ids read at an older revision
stay valid; an id whose word is no longer on the timeline is refused and named, and nothing is cut.

Each cut keeps part of the natural pause: on each side, half the gap to the neighbouring word, at most 30 ms
(tight), 75 ms (balanced, the default) or 160 ms (loose), so the joined pause is at most 60, 150 or 320 ms. The edges
then move to the quietest 10 ms within 40 ms, never into a neighbouring word, and land on the timeline's frame grid.
Each seam gets a 10 ms audio fade. The cut removes the range from the tracks of the cut clips, from sync-locked
tracks and from tracks holding linked items, like `ove edit ripple-delete`; a locked track in the way refuses it
(`ripple.blocked`, naming the item). Caption items lose the cut words and keep the rest in step with the sound; a
caption left with no word is removed. MCP: `remove_words`.

The result has `rev`, `summary`, `ranges` (each `clipId`, `start`, `end` in timeline seconds before the cut, and the
`wordIds`), `removedWords`, `captionsChanged`, `captionsRemoved`, `timeMap` and `diff`.

## Remove dead air

```sh
ove edit remove-silence <project> [--min-pause S] [--padding S] [--relative-db DB] [--dry-run] [--json]
```

Finds pauses with on-device voice activity and cuts those that are at least `--relative-db` (default 12) below the
speaker's own level and at least `--min-pause` long (default 0.5 s, 0.25–3 s). `--padding` (default 0.15 s, 0–0.5 s)
stays on each side of a cut, except at the start and end of a clip. A music bed or loud ambience is kept because it
is not quiet relative to the speech. When no speech is found, nothing is cut and the result says so. Voice activity
needs the inference runtime (install it in Settings > Storage, or `ove models install onnxruntime --yes`); the
speech masks and levels are cached in the project. MCP: `remove_silence`; detection only: `detect_silence` with
`mode: "speech"`.

## Dry run

With `--dry-run` (MCP: `apply: false`) both cutting commands return the same result without changing the project:
`dryRun: true`, the current `rev`, and `diff`, the unified diff of `timeline.tsx` the cut would make. Applying the
same request at the same revision makes exactly that change.
