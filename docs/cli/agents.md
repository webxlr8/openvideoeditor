# Editing OpenVideoEditor projects as an agent

`ove` reads and edits OpenVideoEditor projects. The same commands are available as MCP tools (`ove mcp`). Times are
in seconds. Every change is one revision: the user can undo it in the app, and `ove undo` reverts it.

## The loop

1. **See what the user sees**: `ove app context` — the open project, playhead, selected clip ids. Without the app,
   `ove project list` and `ove project show <project>`.
2. **Read the timeline**: `ove source print <project>` prints `timeline.tsx`, the whole edit as literal TSX:

   ```tsx
   /*
    * … header: aspect, duration, media table (name, id, type, duration), pending count …
    * rev: 1794433052235
    */
   export default (
     <timeline aspect="16:9">
       <track id="t_text" name="Text" kind="video">
         <text id="title" name="Title" start={0.5} end={3.5} textContent="Harbour at dawn" />
       </track>
       <track id="t_main" name="Main" kind="video">
         <clip id="i_1" name="harbour.mp4" start={0} end={4} in={1.5} src="harbour" />
       </track>
       <track id="t_audio" name="Audio" kind="audio">
         <audio id="i_2" name="score.wav" start={0} end={10} src="score" />
       </track>
     </timeline>
   );
   ```

   Elements: `clip` (video), `image`, `audio`, `text`, `caption`. `start`/`end` are timeline seconds, `in`/`out`
   trim the source, `src` names the media (the name from the header's media table, a media id, or a file path).
   Values are literals only (numbers, strings, booleans, arrays, objects) — no expressions. Tracks are listed top to
   bottom: the first track draws over the ones below it. Keep the `rev:` line — it tells ove which revision you
   edited. The project bundle has `timeline.d.ts` next to `timeline.tsx` with every attribute typed.
3. **Edit the text** and save it to a file (or pass it on stdin with `-`): move clips by changing `start`/`end`, trim
   with `in`, add an item by adding an element (the `id` is optional for new items), delete by removing it. Titles:
   `<text start={1} end={4} textContent="Hello" />`. Placeholders for footage you cannot make:
   `<clip start={4} end={8} pending={true} pendingDescription="drone shot of the harbour" />`.
4. **Check, then apply**: `ove source check <project> edited.tsx` reports parse errors with line and column and what
   would change; `ove source apply <project> edited.tsx` applies it as one revision. If the user edited meanwhile,
   both edits are merged; on a real conflict the file's value wins (`--prefer editor` keeps the user's).
5. **Verify**: `ove verify <project>` lists overlaps, gaps, clips running past their source, missing media and
   pending placeholders (exit 1 when there are errors).
6. **Look**: `ove look <project> --at 2,5.5 --out frames/` renders composited frames; `--every 2 --sheet sheet.png`
   makes a contact sheet; `--stats` reports brightness and black/clipped pixels. Frames come from the same
   compositor and backend as export (`--backend auto|cpu|gpu`).

## As MCP tools

Over MCP (`ove mcp`) the same steps are tools: `print_timeline`, then `apply_timeline` with text edits instead of a
file — `{"project": "Demo", "rev": <rev from the header>, "edits": [{"oldText": "…", "newText": "…"}]}`. Each
`oldText` must occur exactly once in the printed text; `newText: ""` deletes. The receipt returns the new `rev` and
the printed lines of what changed, so the next edit needs no new print. `undo_timeline` undoes only your own latest
change; `undo_my_changes` undoes all of this session's changes. `ove docs tools` lists every tool.

## Media

- `ove media add <project> /abs/path/a.mp4 /abs/path/b.wav` probes and places clips (video on Main, audio on Audio).
- `ove media list <project>` gives ids, durations and online/offline status; `ove media probe` the stream details.
- Inspect footage before cutting: `ove media grab <project> <media> --at 3 --out f.png`,
  `ove media filmstrip <project> <media> --out strip.png`, `ove media scenes <project> <media>` (cut times).
- Audio: `ove audio silence` (quiet ranges to cut), `ove audio loudness` (LUFS, true peak), `ove audio levels`,
  `ove audio waveform`, `ove audio duck` (lower music under dialogue with volume keyframes).
- Volume over time: add a keyframe track to a `clip` or `audio` element, for example
  `keyframeTracks={[{property: "volume", keyframes: [{time: 0, value: 100}, {time: 2, value: 0}]}]}` fades out over
  2 s. Values are percent (0–200, like `volume`), times are seconds from the item's start, and the level changes
  evenly in decibels between keyframes. See `ove docs timeline`.

## When the app is open

The app holds the project while it is open. `ove` then sends edits to the app, which applies them at once (the
editor updates, the user can undo them). `ove app seek 12.5`, `ove app play`, `ove app pause` drive the viewer;
`ove screenshot out.png` renders the frame under the playhead.

## Finishing

- `ove export <project> out.mp4 [--height 1080] [--fps 30]` — refused while placeholders are pending.
- `ove timeline export <project> cut.fcpxml` — for Final Cut Pro / Resolve (`.xml` Premiere, `.edl`, `.otio`).

## Rules of thumb

- Read before you write: always edit the text printed for the current revision.
- One logical change per apply, with a clear result — each becomes one undo step.
- Use `--json` for machine-readable output; errors are `{"error": {"code", "message", "details"}}` on stderr.
- Exit codes: 0 ok, 1 refused, 2 usage error, 3 the app is not running (for app commands).
