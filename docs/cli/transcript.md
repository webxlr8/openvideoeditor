# `ove transcript`

The words said on the timeline, read from the transcripts cached in the project (the same transcripts captions
use). Agents call the same tool as `get_transcript`; the app, the command line, MCP and the assistant get the same
words for the same project revision.

```sh
ove transcript <project> [--clips <id,id>] [--segments] [--offset N] [--limit N] [--json]
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
```
