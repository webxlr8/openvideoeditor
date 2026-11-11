# Spec Delta

## Purpose

Previewing a media file in the viewer: opening it, choosing the decode path, transport controls, frame-accurate
seeking, audio output with sync, and the diagnostics used to measure playback quality.

## ADDED Requirements

### Requirement: Open media for preview
The app SHALL open a local video or audio file chosen by file dialog, drag-and-drop onto the window, or open-file
routing, and SHALL show its first frame (or a waveform placeholder for audio-only files) together with duration,
resolution, frame rate, codec and audio layout. Supported for this change: MP4/MOV/MXF/MKV containers with H.264,
HEVC (8- and 10-bit, 4:2:0 and 4:2:2), ProRes (422 Proxy/LT/422/HQ/4444), DNxHD/DNxHR, AV1 and VP9 video, and
AAC/PCM/ALAC/Opus audio.

#### Scenario: Open a 4K ProRes file
- **WHEN** the user drops a 3840×2160 ProRes 422 HQ MOV onto the window
- **THEN** its first frame is visible in the viewer within 500 ms and its properties are shown

#### Scenario: Corrupt or unsupported file
- **WHEN** the user opens a truncated file or an unsupported codec
- **THEN** the app shows which file failed and why, and the previously open file (if any) keeps playing

#### Scenario: File removed while open
- **WHEN** the open file is deleted or its drive is unmounted during playback
- **THEN** playback stops with a "media offline" message and the app keeps running

### Requirement: Decode path selection
The app SHALL decode with the platform hardware decoder when the stream is supported by it, and SHALL fall back to
software decoding automatically, including mid-stream after a hardware error, without user action. The active decode
path SHALL be visible in the diagnostics overlay.

#### Scenario: Hardware decode available
- **WHEN** an HEVC 10-bit 4:2:0 file is opened on a machine whose GPU decodes it
- **THEN** the diagnostics report the hardware path

#### Scenario: Hardware unsupported profile
- **WHEN** a stream is not supported by the hardware decoder (for example HEVC 4:2:2 10-bit on a GPU without support)
- **THEN** playback uses software decoding and the diagnostics report the software path

#### Scenario: Hardware failure mid-stream
- **WHEN** the hardware decoder returns an error during playback
- **THEN** playback continues from the same frame on the software path with at most one visible hitch

### Requirement: Transport and seeking
The viewer SHALL provide play/pause, step one frame forward/back, jump to start/end, scrub, J/K/L shuttle (−2×, −1×,
pause, 1×, 2×), loop, and seek to an exact timecode. Seeking SHALL be frame-accurate: the displayed frame for a
requested time SHALL be the frame whose presentation interval contains that time, for constant and variable frame
rate sources.

#### Scenario: Frame-accurate seek
- **WHEN** the user seeks to frame N of a long-GOP H.264 file
- **THEN** the viewer shows exactly frame N (verified against a reference decode by frame checksum)

#### Scenario: Frame step
- **WHEN** playback is paused and the user presses the step-forward key 10 times
- **THEN** the displayed frame advances by exactly 10 frames

#### Scenario: Variable frame rate source
- **WHEN** a phone-recorded VFR file is opened and played
- **THEN** frames are presented at their own timestamps and the audio stays in sync

### Requirement: Playback performance
On the reference machines, the viewer SHALL play the following at 1× for 60 s with at most 1 dropped frame and at
most 2 frames of average A/V offset: 3840×2160 H.264 and HEVC 10-bit at up to 60 fps, 3840×2160 ProRes 422 HQ at up to
30 fps, and 1920×1080 DNxHR HQ at up to 60 fps. Seek-to-displayed-frame SHALL complete within 150 ms (p95) for
intra-frame codecs and 300 ms (p95) for long-GOP codecs.

#### Scenario: 4K60 HEVC playback
- **WHEN** a 60-second 3840×2160 60 fps HEVC 10-bit file plays at 1×
- **THEN** the diagnostics report ≤ 1 dropped frame and average A/V offset ≤ 2 frames

#### Scenario: Seek latency
- **WHEN** 100 random seeks are issued on a 1-hour long-GOP H.264 file
- **THEN** the 95th percentile time from request to displayed frame is ≤ 300 ms

### Requirement: Audio output and sync
The app SHALL play the file's audio on the system default output device, using the audio clock as the master clock
during playback, with volume and mute controls. When the default output device changes, playback SHALL continue on
the new device within 1 s.

#### Scenario: Output device change
- **WHEN** the user connects headphones during playback
- **THEN** audio continues on the headphones within 1 s and video stays in sync

#### Scenario: No audio device
- **WHEN** no audio output device is available
- **THEN** video plays using the system clock and the UI shows that audio is unavailable

### Requirement: Playback diagnostics and benchmark
The app SHALL provide a diagnostics overlay (toggleable) showing decode path, frame size and format, presented and
dropped frames, A/V offset, seek latency and process memory. A headless benchmark command SHALL play and seek a given
file without UI and write the same metrics as machine-readable JSON.

#### Scenario: Benchmark output
- **WHEN** the benchmark command runs on a file
- **THEN** it exits 0 and writes JSON containing decode path, dropped frames, A/V offset statistics, seek latency
  percentiles and peak memory
