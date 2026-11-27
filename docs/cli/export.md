# `ove export`

Renders the whole timeline to an MP4 file (H.264 video + AAC audio). Available when the CLI is built with the `export`
feature (`cargo build -p ove-cli --features export`; desktop bundles include it).

```sh
ove export <project> <out.mp4> [--height <px>] [--fps <n>] [--loudness off|-14|-16|-23|-24] [--json]
ove export <project> <out.m4a|.wav|.flac> --audio-only [--format m4a|wav|flac] [--loudness …] [--json]
```

- `--height` (default 1080): output height; the width follows the project's aspect ratio (1920×1080 for 16:9 at
  1080, 608×1080 for 9:16, …), always even.
- `--fps` (default 30).
- Refused (exit 1, code `export.refused`) while the timeline has pending placeholders, or when it is empty.
- `--audio-only` writes the mix alone and renders no video: `m4a` (AAC, with an edit list so players skip the
  encoder's priming and the file has the timeline's duration), `wav` (24-bit PCM, the mixer's samples rounded once)
  or `flac` (24-bit; needs an FFmpeg build with the FLAC muxer). `--format` picks it; without it the output's
  extension decides, else `m4a`. The same rules as video export apply: refused while placeholders are pending or
  media is offline, and nothing is left behind when refused or cancelled.
- `--loudness` normalises the mix to an integrated loudness target: −14 (streaming), −16 (podcasts), −23 (EBU R128)
  or −24 LUFS (ATSC A/85). Export first measures the mix (audio only, no video), then applies one static gain before
  the true-peak limiter, so the file measures within 0.5 LU of the target with its true peak at or under −1 dBTP. A
  mix that the limiter would make quieter (loud short peaks) is measured again with the gain until it lands on the
  target. A silent timeline exports without gain. The JSON result gains `"loudness": {"targetLufs", "beforeLufs",
  "beforeTruePeakDbtp", "afterLufs", "afterTruePeakDbtp", "gainDb", "silent"}` (`beforeLufs`/`afterLufs` are `null`
  for silence).
- Every export, with or without a target, keeps the mix's true peak at or under −1 dBTP (look-ahead true-peak
  limiter; quieter audio passes through unchanged).
- `ove audio loudness <project> --timeline` measures the timeline mix without exporting.

Video encoder, first that opens: VideoToolbox (macOS); NVENC, Quick Sync, AMF, Media Foundation (Windows); OpenH264;
libx264 (development FFmpeg builds only — GPL, never shipped); MPEG-4 Part 2. Audio: FFmpeg's AAC, 192 kb/s stereo
48 kHz.

Output: without `--json`, progress on stderr and one summary line. With `--json`, one JSON line per percent
`{"progress": {"frame", "totalFrames", "fraction"}}` and a final object
`{"path", "width", "height", "frames", "durationSeconds", "videoEncoder", "secondsTaken"}`.

What renders (v1): video and image layers with trim, speed, reverse, transform, crop, opacity, rounded corners, blend
modes (normal, multiply, screen, darken, lighten, add), keyframes (x, y, scale, rotation, opacity), fades and
crossfades, adjustment layers and per-item `colorGrading`; text items (Inter regular/bold, size, colour, alignment,
line height, letter spacing, stroke, shadow, background box); caption items (active segment, highlighted active
word). Not yet: code and shader items (change 4), effects, LUTs and `colorGrade` wheels/curves (change 12), text
animations (change 11), Gaussian blur, audio crossfades and pitch-preserving speed.
