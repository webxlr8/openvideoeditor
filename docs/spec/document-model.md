# Document model

The project document is the single source of truth that every editor (GUI, built-in agent, external agent, CLI, MCP)
changes through one pipeline: print → edit → check → apply.

## Bundle layout

```
<Name>.<bundle>/
  project.json        canonical document (only the app writes it)
  timeline.tsx        literal TSX projection, regenerated on every save; parsed, never executed
  timeline.d.ts       typings for the projection (written by the app)
  AGENTS.md           house rules for agents working in this bundle
  media/              media owned by the bundle (others are referenced in place)
  cache/              derived data per media hash: thumbnails, waveforms, peaks, album-art, audio, meta,
                      transcripts, mattes, cards (+ summary.json, probe.json, converted-probe.json, source-stamp)
  proxies/            edit proxies (regenerable)
  .revs/<rev>.json    revisions for history/undo
  agent/chats.json    chat index; agent/chats/<id>.jsonl one record per line
  recipe/             template copied in when a project starts from one
  .scratch/           agent working files      .context/  files pasted into chat
```

Library root: `~/Movies/<App>/accounts/<sha256(accountId)>/`, overridable by an env var or config; network drives are
refused. A single-file archive (`.<archive>`) packs the bundle with referenced media and drops regenerable dirs.
Bundle folder names are sanitised (no `\/:*?"<>|`, control chars, leading dots or trailing dots/spaces; 120 chars max;
fallback "Untitled Project") and never change on rename.

## project.json

```
{ format, version,
  project: { id, name, createdAt, updatedAt, stateUpdatedAt(ms, = rev), aspectRatio, temp?, mediaBaseDirectory?,
             media: [MediaItem], timelineItems: [Item], tracks (field name inferred), cast?: [CastMember] },
  media:   [{ mediaId, nativePath, ownedByBundle }] }
```

MediaItem `{ id, createdAt, addedAt, data: { type, src?, filePath?, fileName, format, size, duration, width, height,
frameRate, hasAudio, thumbnailIds, waveformId, albumArtId, *GenerationStatus, conversionStatus, transcriptStatus,
transcriptLanguage, generation?, presetId?, content?, code? } }`.

Track `{ id, name, order, type: video|audio|caption|adjustment|layout, locked, visible, muted }`. First track = top layer.

## Items

Common: `id, trackId, type, name, startTime, endTime, trimStart, trimEnd, mediaItemId, pending, transform{x,y,scale,
rotation,flipX}` (pixels from frame centre), `crop{top,right,bottom,left}` (%), `opacity, hidden, zIndex, blendMode,
keyframeTracks[{property: x|y|scale|rotation|opacity|gaussianBlur|borderRadius, keyframes[{time (item-relative), value}],
easing}], transitionIn/Out{type|presetId, duration, …}, effects[{id,type,…}], borderRadius, gaussianBlur,
captionComposition{mode: front|behind-subject, fallback: front|hide}, autoReframe`.

| type | projection tag | extra fields |
|---|---|---|
| video | `<clip>` | sound: volume 0–200, muted, audioFadeIn/Out{duration,easing}, playbackSpeed, playbackDirection; grade |
| image | `<image>` | grade |
| audio | `<audio>` | sound fields |
| text | `<text>` | textContent, fontFamily/Size/Weight/Style, color, textAlign, lineHeight, letterSpacing, stroke*, shadow*, background{Color,Opacity,Padding}, textAnimation{preset,…} |
| caption | `<caption>` | captionText, captionSegments[{id,text,startTime,endTime,wordTimings[{word,start,end}]}], captionPresetId, captionPosition, captionFontSize/Family, captionTextColor, captionHighlightColor |
| code | `<code>` | codeComposition: {source,width,height,assets{name: src or {src,in,loop}}} or {template,props,width,height,assets} — a pure JSX function of {time,width,height,assets}, linted on apply |
| adjustment | `<adjustment>` | adjustmentKind, lutPresetId, lutIntensity, grade |
| shader | `<shader>` | shaderComposition: {source or template, params, controls{label,min,max,step,format,toggle}, amount, fadeIn, fadeOut, region: all|subject|background, defaults} |

Grade = `colorGrading{exposure,brightness,contrast,highlights,shadows,temperature,tint,saturation,vibrance,gamma}` +
`colorGrade{wheels{lift,gamma,gain,offset: {r,g,b,y}}, curves{master,red,green,blue,hueVsSat,hueVsHue,hueVsLum,lumVsSat},
saturation, lut{preset|file,intensity,stage: input|look}, match: ref | {reference,at,strength} | {frame,strength}}`.
`lut.file` paths are copied into `luts/` on apply; `match` is fitted on apply and replaced by concrete values.

Aspects: 16:9, 4:3, 3:4, 9:16, 1:1, 4:5.

## Projection rules

- Tag map: clip↔video, others same name. Renames: start↔startTime, end↔endTime, in↔trimStart, out↔trimEnd, kind↔type.
- Attribute values are literals only (string, number, boolean, array, object). No expressions; comments dropped.
- Quoted attributes keep backslashes literally; escapes need braces.
- `src` resolves a media name from the header table, a media id, or a path (absolute, `./`, `../`, `~/`); paths are
  ingested on apply and rewritten to ids.
- Unknown attributes round-trip untouched; unknown keys inside known objects warn.
- Header: aspect, duration, counts, media table, cast, diagnostics, pending count, `rev`.
- Clip/image without transform/keyframes/`autoReframe={false}` gets auto-reframe: fill frame height, centred crop.
  Changing aspect re-fills auto-reframed clips on the first video track and layout tracks.
- Scripts: `apply --eval` transpiles and runs a `.tsx` file in the CLI process with a JSX factory producing the same tree.

## Apply / merge

1. Parse → diagnostics (errors refuse the whole apply; warnings apply and are listed in the header).
2. Diff against the revision named by `rev` → ops. 10 op kinds: `addTrack, updateTrack, removeTrack, reorderTracks,
   unsetTrackFields, addItem, updateItem, removeItems, unsetItemFields, setAspectRatio`.
3. Rebase ops onto the current document item by item; GUI edits to other items survive. Same-item conflicts: file wins
   by default, `--prefer editor` keeps GUI values; conflicts are reported.
4. Items without `id` are new; items missing from the file are removed (reported as "removed N items"); a changed id
   means replace (loses undo trail).
5. New revision written to `.revs/`; open app applies live (~200 ms) as one undo step.
6. `pending` items render nothing, block export, and are counted by verify; filling keeps the id.

MCP variant: `apply_timeline({project, rev, edits:[{oldText,newText}]})` — each oldText unique in the printed file,
non-overlapping; `newText: ""` deletes; anchor + new JSX inserts.

## Verify (lint)

Overlaps, gaps, off-screen text, clips past their source (holds last frame), missing media, pending items, invalid
compositions.

## Chat log records (`agent/chats/<id>.jsonl`)

`user{id,text}`, `session{sessionId,model}`, `assistant{messageId, blocks[text|tool_use{id,name,input}]}`,
`tool-result{toolUseId,text}`, `question{id, questions[{header,question,multiSelect,options[{label,description}]}]}`,
`result{isError,durationMs,costUsd,text,threadUsage{inputTokens,cachedInputTokens,…},time,note?}` (one per turn,
successful or not; `note` names the limit a turn stopped at), `revisions{turn,revs[]}` (project revisions a tool call
of the turn starting at user record `turn` made), `turn-undone{turn,rev}` ("Undo this turn" reverted them as `rev`),
`summary{firstKept,text,time}` (the turns before the `firstKept`-th user record are sent to the model as this
summary; the records stay for display).
