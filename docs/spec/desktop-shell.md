# Desktop shell

- Window: hidden-inset title bar, custom traffic-light position (14–16, 14), dark only. Browser-style tab strip; each
  tab (Home, Settings, one per project) is its own renderer view; idle tabs sleep and wake on any command; a hidden
  pre-warmed renderer speeds the next open. Only the front tab plays audio.
- URL scheme `<app>://` opens a template on Home. Document types: project bundle (package, owner), single-file project
  archive (owner), Premiere `.prproj` (viewer, alternate → import).
- Second instance / open-file routes paths to the running app.
- App menu: New Tab ⌘T, Close Tab ⌘W, Next/Previous Tab, Settings… ⌘,, Check for Updates…, Sign In/Out…, Developer ›
  Reset Onboarding; File › New Project ⌘N, Open Project… ⌘O, Save Project As… ⌘⇧O, Projects, Install Agent Tools…,
  Close Window ⌘⇧W; Edit (standard roles); Playback; View (fullscreen); Window; Help › Keyboard Shortcuts… ⌘/,
  Report an Issue… ⌘⇧B, Website.
- Fuses: RunAsNode on (the CLI runs as `ELECTRON_RUN_AS_NODE=1 <App> cli.mjs`), embedded ASAR integrity on,
  only-load-from-ASAR on, NODE_OPTIONS and --inspect off, cookie encryption off.
- Entitlements: app — JIT, unsigned executable memory, user-selected file read-write, microphone, camera. Agent runtime —
  JIT, unsigned executable memory, disable library validation, Apple Events, microphone.
- Bundled tools: ffmpeg, ffprobe, uv (provisions a pinned Python with numpy, opencv-headless, pillow, scipy for agent
  scripts), yt-dlp resolved from system or downloaded. "Install Agent Tools" links the CLI into `~/.local/bin` and installs
  the agent skill into Claude Code / Codex / Cursor skill dirs.
- Updates: own update endpoint, Squirrel.Mac, channels alpha / beta / stable; banners "downloading", "restart to update",
  "needs an update".
- Telemetry: product analytics (opt-out in Settings › Privacy) and crash reporting.
- Auth: OAuth in the system browser with code paste-back; CLI device flow; tokens stored in an app-support JSON (0600).
- Plan gating flags fetched per account: exportLimit, agentTimeMs, maxExportShortSide, watermark, nleExport,
  stockFootage, webFootage, directLinks, referenceFromVideo, voiceCloning, translation, models (none|base|premium|all).
