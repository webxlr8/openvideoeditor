# The running app: `ove app`, `ove screenshot`

```sh
ove app context
ove app open <project>
ove app seek <seconds>
ove app play
ove app pause
ove screenshot <out.png> [--width <px>]
```

The app publishes a control link while it runs: a loopback port and a random token in
`<library>/.ove-control.json`, readable only by your user. `ove` finds the app through the library it uses, so both
must point at the same library (`--library` / `OVE_LIBRARY`).

**context** returns the open project (name, id, bundle, revision), the playhead in seconds, whether it is playing,
the selected clip ids and the viewer mode. **screenshot** renders the composited frame under the playhead at
`--width` (default 1280).

**Edits while the app has the project open.** The app holds the project's write lock. Every editing command
(`source apply`, `undo`, `media add|remove|relink`, `project rename`, `timeline import`) then sends its change to the
app with the revision it was computed from; the app applies it as one revision (source `cli` or `mcp`), the editor
updates and the change can be undone there. If the user edited in between, the command recomputes and retries (up to
three times). Text output ends with "(in the app)" and JSON has `"viaApp": true`.

**Spending.** An agent's generation request (`generate_*` with `confirm`) asks the app over the control method
`confirm_spend`: the app shows a price card and answers with the person's choice, to that request only. Without the
app, `ove mcp` uses the MCP client's confirmation prompt, or refuses with `confirm.required`.

**Live activity.** While the app has a project open, its timeline shows what agents do to it. Changes sent through
the app (above) are shown from the revisions they make: the changed items are outlined, with a badge naming the
client. Reads are reported with the control method `notify` (`{bundle, actor, activity: {kind: "read", ids?, start?,
end?}}`): `print_timeline` with `from`/`lines` reports the items on those lines, a full print the whole timeline, `look`
the times it renders and `get_transcript` the clips it reads. `notify` is fire and forget: without the app, or with an
app that does not know it, nothing happens.

Exit code 3 means the app is not running.
