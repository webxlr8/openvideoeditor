# Proposal: add-captions

## Why

Captions are expected on almost every social video and must work offline.

## What Changes

- `ove-transcribe`: whisper models (tiny/base/small) downloaded on first use with sha256 check; 16 kHz decode through ove-media; word timestamps; transcripts cached per media file.
- 21 caption styles of our own design with word marks (colour, box, ring, underline) and animations.
- Auto-captions as one revision aligned through each clip's trim, speed and direction; Captions panel with style gallery and transcript editor.
