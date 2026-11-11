# `ove export`

Renders the whole timeline to an MP4 file (H.264 video + AAC audio). Available when the CLI is built with the `export`
feature (`cargo build -p ove-cli --features export`; desktop bundles include it).

```sh
ove export <project> <out.mp4> [--height <px>] [--fps <n>] [--json]
```

- `--height` (default 1080): output height; the width follows the project's aspect ratio (1920×1080 for 16:9 at
  1080, 608×1080 for 9:16, …), always even.
- `--fps` (default 30).
- Refused (exit 1, code `export.refused`) while the timeline has pending placeholders, or when it is empty.

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
