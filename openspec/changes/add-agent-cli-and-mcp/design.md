# Design: add-agent-cli-and-mcp

## Control link

Loopback TCP rather than a Unix socket / named pipe: one implementation for macOS, Windows and Linux. Security comes
from binding to 127.0.0.1, a 192-bit random token compared in constant time, and the endpoint file being readable
only by the user (mode 0600 on Unix; the library folder's ACL on Windows). The token changes every launch and the
file is removed on exit; a stale file is harmless because nothing answers on its port (or a new app wrote a new
token). Protocol: one JSON line request `{token, method, params}`, one JSON line reply `{ok, result | error}`.

Discovery is by library: `ove` reads the endpoint in the library it resolves (`--library`, `OVE_LIBRARY`, platform
default), so the CLI only ever talks to the app serving the same projects.

## Edits through the app

`Writer::open` takes the write lock when it can. When the lock is held and the app reports the same bundle open, the
command reads the bundle read-only (revision records are durable, so the read sees the app's latest revision),
computes its ops exactly as it would locally, and sends `apply {bundle, expectRev, ops, summary, source}`. The app
refuses with `rev.changed` when its revision moved; `edit()` recomputes from a fresh read and retries up to three
times. Undo sends `restore {rev}`. The app emits the usual project-changed event, so the editor, viewer and undo stack
follow. Diagnostics (`set_diagnostics`) are only written locally — the app recomputes its own.

## MCP

Tools are a table mapping JSON arguments to command-line arguments; the server parses them with the same clap
definitions and runs the command in-process with `--json`. One code path means the tools, the CLI and the docs cannot
drift. Text-file arguments (edited timeline source) are written to a temporary file; image outputs go to a temporary
file or folder and come back as `image` content. Progress output is suppressed while serving (stdout carries only
protocol messages). Protocol versions 2025-06-18, 2025-03-26 and 2024-11-05 are accepted.

## Analysis

`ove-analyze` decodes through `ove-media` and renders timeline frames with `ove-render`'s `Renderer`, so a look is
exactly what export produces. Loudness implements BS.1770-4 K-weighting for the actual sample rate, 400 ms blocks with
75 % overlap, absolute (−70 LUFS) and relative (−10 LU) gates, LRA over 3 s windows (−20 LU gate, 10th–95th
percentile), and a 4× oversampled true-peak estimate. Verified against FFmpeg's `ebur128` filter.

## Media feature

Commands that read media need FFmpeg; they are behind the `media` feature (`export` is kept as an alias). Builds
without it refuse those commands with `media.unavailable`; installers build the CLI with it.
