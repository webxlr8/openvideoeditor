# IPC surface

Renderer ↔ main uses one bridge with three channels (`main:request`, `main:response`, `main:event`).
Payloads are schema-validated; a router maps a channel name to a handler and queues requests until the main
process marks itself ready. Preload exposes only `{ platform, getPathForFile, send, on }`.

## Request handlers (105)
- **access**: check, get
- **account**: get, plan, plan-features, sign-in, sign-in-cancel, sign-in-copy-link, sign-in-reopen, sign-out
- **agent-setup**: done
- **agent-tools**: install
- **app**: about, version
- **cli**: status
- **cloud**: fetch
- **control**: reply
- **edit**: native-command
- **export**: log, sink, sink-read
- **feedback**: context, recording, recording-stop, submit
- **file**: preview, reveal
- **library**: call, choose, info, move, move-cancel, retry, reveal, status
- **local-agent**: answer, chat-close, chat-delete, chat-new, chat-open, chat-rename, chat-skills, chats, dequeue, agent-advisory, agent-advisory-get, install, login, login-code, login-code-page, logout, send, set-enabled, status, steer-queued, stop, transcript
- **media**: add, preview, relink, relink-folder, unregister
- **project**: patch, rename, reveal, suggest-name
- **recipes**: cached, fetch, install, remember
- **reference**: import, inspect
- **renderer-diagnostics**: get-settings, process-metrics, set-settings, version-click
- **skills**: status
- **tabs**: activate, close, command, home, list, move, open, overlay, peek, settings
- **telemetry**: capture, defect, feedback, feedback-available, get-setting, set-setting
- **thanks**: get, said
- **update**: check, dismiss, install, recall-get, status
- **welcome**: get, seen
- **window**: is-fullscreen, set-activity, set-editor-state

## Events main → renderer (11)

`control:request`, `editor:get-more-credits`, `editor:set-aspect-ratio`, `feedback:recording-stop`, `home:recipe`, `home:section`, `menu:action`, `settings:open`, `shell:navigate`, `tabs:modifier`, `update:notice`

## Library worker (separate utility process)

Reached through `library:call {method, args}`; only this allow-list is exposed to the renderer:

`projects.list`, `projects.summaries`, `projects.posterCards`, `projects.usage`, `projects.freeSpace`, `projects.read`, `projects.write`, `projects.trash`, `projects.saveContextFile`, `media.locate`, `cache.get`, `cache.put`, `cache.delete`, `cache.keys`, `cache.clear`, `transcripts.ensure`, `mattes.ensure`, `audioProcessing.process`, `voiceovers.clone`, `timeline.pendingGenerationKeys`, `timeline.retireGenerationKey`, `timeline.finishGeneration`, `luts.list`, `luts.read`, `luts.add`

Internal worker API also covers: projects create/open/openExisting/copyOpen/openArchive/saveArchive/rename/updateScoped,
Premiere sequence listing + import, bundle path lookup, media register/relink/relinkFromFolder/unregister/resolvePath/
mirror/isOwned, HEIC preview, proxy path/ensure, recipe install, context-file save.

## Control socket (CLI → running app)

JSON-RPC 2.0, newline-delimited, over a unix socket in the app's userData dir (`app.sock`, mode 0600). A random
24-byte hex token is written to a sibling token file (0600); the first request must authenticate with it. Methods:

`app.context`, `project.open`, `player.seek`, `player.play`, `player.pause`, `frame.render`, `code.render`, `code.probe`, `window.screenshot`, `export.start`, `export.cancel`

Most take an optional `project` to target a background tab; the app forwards to that tab's renderer and awaits a reply
(`control:request` / `control:reply`).
