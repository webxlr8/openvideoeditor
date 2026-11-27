# `ove export`

Renders the timeline, or a range of it, to a video or audio file. Available when the CLI is built with the `media`
feature (`cargo build -p ove-cli --features media`; desktop bundles include it). The MCP tool `export_video` takes the
same settings.

```
ove export <project> <out> [--preset <name>] [--format <container>] [--video-codec <codec>] [--profile <p>]
           [--bit-depth <8|10|12>] [--quality <level>] [--alpha] [--audio-codec <codec>] [--audio-bit-depth <16|24>]
           [--range <start-end>] [--size <height|match>] [--height <px>] [--fps <n>] [--audio-only]
           [--loudness off|-14|-16|-23|-24] [--detach] [--json]
```

## Settings

An export is described by one settings value. A preset gives a complete one; the flags then change single fields.
Without a preset, the format (or the extension of `<out>`) picks the container and its usual codecs: H.264 and AAC
for MP4, MOV and MKV; VP9 and Opus for WebM; AAC for M4A; 24-bit PCM for WAV; 24-bit FLAC for FLAC.

| Field | Values |
|---|---|
| container (`--format`) | `mp4`, `mov`, `mkv`, `webm`, `m4a`, `wav`, `flac`. The extension of `<out>` must match. |
| video codec | `h264`, `hevc`, `prores`, `dnxhr`, `vp9`, `av1` |
| profile | ProRes: `proxy`, `lt`, `standard`, `hq` (default), `4444`. DNxHR: `lb`, `sq`, `hq` (default), `hqx`, `444`. HEVC: `main`, `main10`. |
| bit depth | H.264: 8. HEVC: 8 or 10. ProRes: 10. DNxHR LB/SQ/HQ: 8; HQX/444: 10. VP9, AV1: 8 or 10. |
| quality | `very-low`, `low`, `medium` (default), `high`, `very-high`. Sets the data rate of H.264, HEVC, AV1 (bit rate) and VP9 (constant quality). ProRes and DNxHR take theirs from the profile. |
| alpha | ProRes 4444 in MOV, and 8-bit VP9 in WebM only. Areas with nothing on them are transparent. |
| audio codec | `aac` (MP4, MOV, MKV, M4A), `pcm` (MOV, MKV, WAV), `flac` (MKV, FLAC), `opus` (MKV, WebM), `none`. 48 kHz stereo. |
| audio bit depth | PCM and FLAC: 16 or 24 (default 24). |
| range | `START-END` in seconds; only that part is rendered. The end must be after the start and inside the timeline. |
| size | A height (`720`, `1080` (default), `2160`, `4k`); the width follows the aspect ratio. `match`: the timeline's own size (1920×1080 for 16:9, 1080×1920 for 9:16, …). |
| fps | 1–120 (default 30; presets use the timeline's rate). |

Combinations that cannot be written are refused before anything renders (exit 1, code `export.invalid`), with a
message that names the conflicting fields: ProRes in MP4, alpha with H.264, 10-bit H.264, a range that ends before it
starts, and so on.

Presets (names match ignoring case, spaces and punctuation, so `--preset prores-422-hq-master` works):

| Preset | What it writes |
|---|---|
| YouTube 1080p | H.264 and AAC in MP4, 1080 pixels high, high quality |
| YouTube 4K | H.264 and AAC in MP4, 2160 pixels high, high quality |
| Social 720p | H.264 and AAC in MP4, 720 pixels high |
| HEVC 10-bit | HEVC Main 10 and AAC in MP4 at the timeline's size (hardware encoder needed) |
| ProRes 422 HQ master | ProRes 422 HQ and 24-bit PCM in MOV at the timeline's size |
| ProRes 422 Proxy | ProRes 422 Proxy and 24-bit PCM in MOV, for offline editing |
| ProRes 4444 with alpha | ProRes 4444 with a transparent background and 24-bit PCM in MOV |
| DNxHR HQ master | DNxHR HQ and 24-bit PCM in MOV at the timeline's size |
| DNxHR HQX 10-bit | DNxHR HQX and 24-bit PCM in MOV at the timeline's size |
| Web VP9 | VP9 and Opus in WebM, 1080 pixels high |
| Web VP9 with alpha | VP9 with a transparent background and Opus in WebM |
| AV1 (hardware) | AV1 and AAC in MP4, 1080 pixels high (hardware encoder needed) |
| Audio WAV 24-bit | The mix as 24-bit 48 kHz WAV |
| Audio WAV 16-bit | The mix as 16-bit 48 kHz WAV |
| Audio M4A | The mix as AAC in M4A |
| Audio FLAC | The mix as 24-bit FLAC |

MCP and the app can also pass a whole settings value as JSON (`export_video` argument `settings`):

```json
{"container": "mov", "video": {"codec": "prores", "profile": "4444", "alpha": true},
 "audio": {"codec": "pcm", "bitDepth": 24}, "size": "match", "fps": "match", "range": [10, 20]}
```

## Encoders

Each codec has a list of encoders, best first; the first one this computer can open is used, and the result names it
(`videoEncoder`, `encoderLabel`) with any that were tried before it (`encoderFallbacks`).

- H.264: VideoToolbox (macOS); NVENC, Quick Sync, AMF, Media Foundation (Windows); OpenH264; MPEG-4 Part 2 as a
  last resort (the result then carries `encoderWarning`).
- HEVC: VideoToolbox (macOS); NVENC, Quick Sync, AMF, Media Foundation (Windows). There is no software HEVC encoder:
  without one of these, the export is refused before rendering and suggests ProRes or H.264.
- AV1: NVENC, Quick Sync or AMF hardware encoders only; refused otherwise, suggesting VP9 or H.264.
- ProRes: VideoToolbox on macOS, else FFmpeg's encoder (named "ProRes (FFmpeg encoder)": it is not Apple-certified).
- DNxHR: FFmpeg's encoder. VP9: libvpx. Audio: FFmpeg's AAC and FLAC encoders, libopus; PCM and WAV are written
  directly.

HDR: with `hdr` (`pq` or `hlg`) in the settings, HEVC 10-bit and ProRes exports are tagged BT.2020 with that transfer
and mastering metadata. The timeline does not produce HDR frames yet, so such an export is refused with "HDR timelines
are not available yet".

## Audio only and loudness

- `--audio-only` writes the mix alone and renders no video: `m4a` (AAC, with an edit list so players skip the
  encoder's priming and the file has the timeline's duration), `wav` (24-bit PCM, the mixer's samples rounded once)
  or `flac` (24-bit; needs an FFmpeg build with the FLAC muxer). `--format` picks it; without it the output's
  extension decides, else `m4a`. MCP: `export_video` with `audioOnly` (`m4a`, `wav`, `flac` or empty). The same rules
  as video export apply: refused while placeholders are pending or media is offline, nothing left behind when refused
  or cancelled.
- `--loudness` (MCP `loudness`) normalises the mix to an integrated loudness target: −14 (streaming), −16 (podcasts),
  −23 (EBU R128) or −24 LUFS (ATSC A/85), with audio-only or video exports. Export first measures the mix (audio
  only, no video), then applies one static gain before the true-peak limiter, so the file measures within 0.5 LU of
  the target with its true peak at or under −1 dBTP. A mix that the limiter would make quieter (loud short peaks) is
  measured again with the gain until it lands on the target. A silent timeline exports without gain. The JSON result
  gains `"loudness": {"targetLufs", "beforeLufs", "beforeTruePeakDbtp", "afterLufs", "afterTruePeakDbtp", "gainDb",
  "silent"}` (`beforeLufs`/`afterLufs` are `null` for silence). Queued exports carry the target and report it in the
  job result.
- Every export, with or without a target, keeps the mix's true peak at or under −1 dBTP (look-ahead true-peak
  limiter; quieter audio passes through unchanged).
- `ove audio loudness <project> --timeline` measures the timeline mix without exporting.

## Examples

```sh
ove export Title title.mp4  # exit 0
ove export Title master.mov --preset prores-422-hq-master  # exit 0
ove export Title overlay.mov --preset prores-4444-with-alpha --size 720  # exit 0
ove export Title middle.mov --video-codec prores --profile lt --range 0.5-1.5 --size 360  # exit 0
ove export Title mix.wav --preset audio-wav-24-bit  # exit 0
ove export Title mix.m4a  # exit 0
ove export Title wrong.mp4 --video-codec prores  # exit 1
ove export Title wrong.mp4 --alpha  # exit 1
ove export Title wrong.mov --range 2-1  # exit 1
ove export Title wrong.mov --preset no-such-preset  # exit 1
ove export Title later.mov --preset prores-422-hq-master --size 360 --detach  # exit 0
ove export queue  # exit 0
ove export presets  # exit 0
ove export status x_unknown  # exit 1
ove export cancel x_unknown  # exit 1
ove export Title --as-media --range 0.5-1  # exit 0
ove export Title --as-media --range 1-1.5 --replace --name Title-card  # exit 0
ove export Title loud.wav --audio-only --loudness -14  # exit 0
ove export Title loud.mp4 --audio-only  # exit 1
```

## Save a range as media

`ove export <project> --as-media --range START-END [--replace] [--tracks a,b] [--name N] [--small]` renders the range
at the project's own size and frame rate into the project's `media/rendered/` folder and adds it to the media library
as media the project owns, in one revision (MCP: `save_range_as_media {project, start, end, replace, tracks, name,
small}`). The file is DNxHR HQ in MOV, or ProRes 422 HQ where DNxHR is not available and on macOS, with 24-bit PCM;
`--small` writes H.264 and AAC in MP4.

- `--replace` (render in place) also replaces what the range covers on the chosen tracks (default: every track that
  is not an audio track) with one clip of the new media, in the same revision; one undo restores the items, and the
  rendered media stays in the library. Only the chosen tracks are rendered; when another visual track has content in
  the range, the render keeps transparency (ProRes 4444) so that track still shows through.
- If rendering fails (for example a media file is offline) nothing changes: no revision, no media, no file.

## The export queue

Exports run one at a time, in the order they were queued, in a worker process of their own that keeps running when
the app window or the terminal closes. The queue is shared: the app's Exports panel, `ove` and MCP agents see the same
jobs. Jobs are files in the app's data folder (`exports/<job-id>.json`; `OVE_EXPORT_QUEUE_DIR` moves it).

- A job renders the project exactly as it was when it was queued: later edits do not change it. If that revision is
  no longer available when the job starts, the job fails and says to export again.
- `ove export <project> <out> …` queues the export and waits for it, printing progress. `--detach` returns the job id
  at once.
- `ove export queue [--all]` lists the jobs (queued, running, and finished in the last day). `ove export status <id>
  [--wait]` shows one; `--wait` returns when it has finished. `ove export cancel <id>` cancels a queued job, or stops a
  running one within 2 seconds and removes its partial file; a finished job cannot be cancelled and its file stays.
  Unknown ids exit 1 with `job.unknown`.
- The file appears at its path only when the job succeeds (it is written as `name.partial.ext` first).
- If the worker stops (the computer restarts, the process is killed), the next `ove export` command or the app puts
  the job back in the queue and starts a worker; the job restarts from the beginning. After 3 attempts it fails.
- MCP: `export_video` returns `{jobId, status}` at once (or the result with `wait: true`); `job_status {job_id, wait}`,
  `cancel_job {job_id}` and `list_export_jobs` work on the same jobs. The requester (`requestedBy`) is the MCP
  client's name, `cli`, `assistant` or `person`.

## Output

Without `--json`, progress on stderr and one summary line. With `--json`, one JSON line per percent
`{"progress": {"frame", "totalFrames", "fraction", "encoder"}}` and a final object `{"path", "container", "width",
"height", "frames", "fps", "durationSeconds", "range", "videoFormat", "videoEncoder", "encoderLabel", "audioEncoder",
"encoderFallbacks", "encoderWarning", "secondsTaken"}`. Audio-only exports have width and height 0 and no video
fields.

Refused (exit 1) while the timeline has pending placeholders, when it is empty, when media it uses in the range is
offline (`export.refused`), and for settings that cannot be written (`export.invalid`, with the conflicting `fields`
in the error details).

What renders (v1): video and image layers with trim, speed, reverse, transform, crop, opacity, rounded corners, blend
modes (normal, multiply, screen, darken, lighten, add), keyframes (x, y, scale, rotation, opacity), fades and
crossfades, adjustment layers and per-item `colorGrading`; text items (Inter regular/bold, size, colour, alignment,
line height, letter spacing, stroke, shadow, background box); caption items (active segment, highlighted active
word). Not yet: code and shader items (change 4), effects, LUTs and `colorGrade` wheels/curves (change 12), text
animations (change 11), Gaussian blur, audio crossfades and pitch-preserving speed.
