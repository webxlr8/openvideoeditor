# Spec Delta

## Purpose

The command line and MCP tools agents use to edit projects, inspect media and the timeline, and work with the running
app.

## ADDED Requirements

### Requirement: Command tree with JSON output
Every `ove` command SHALL accept `--json` and print one JSON object on stdout on success, or
`{"error": {"code", "message", "details"}}` on stderr on refusal. Exit codes SHALL be 0 ok, 1 refused, 2 usage error,
3 the app is not running (for commands that need it).

#### Scenario: Refusal shape
- **WHEN** `ove project create X --aspect 2:1 --json` runs
- **THEN** it exits 1 and stderr holds an error object with code `argument.invalid`

### Requirement: Edits reach the running app
When the app holds a project's write lock and has that project open, editing commands SHALL send their change to the
app with the revision it was computed from, and the app SHALL apply it as one revision of its session with the
command's source. A change computed from an older revision SHALL be refused with `rev.changed` and recomputed.

#### Scenario: Apply while the app is open
- **WHEN** the app has project P open and `ove source apply P edited.tsx` runs
- **THEN** the app's document has the edit as a new revision with source `cli`, the editor shows it, and the command
  reports `viaApp: true`

#### Scenario: Concurrent edit
- **WHEN** the app's revision changes between the command's read and its apply
- **THEN** the first apply is refused with `rev.changed` and the command succeeds on a recomputed retry

#### Scenario: Locked by something else
- **WHEN** another process holds the lock and no app answers for that project
- **THEN** the command is refused with `project.locked`

### Requirement: Control link
The app SHALL listen only on the loopback interface, publish its port and a per-launch random token in
`<library>/.ove-control.json` readable only by the user, reject requests with a wrong token, and remove the file on exit.

#### Scenario: Wrong token
- **WHEN** a request carries a token other than the published one
- **THEN** it is refused with `token.invalid`

### Requirement: MCP server
`ove mcp` SHALL serve the commands as MCP tools over stdio. Each tool SHALL run the matching command, return its JSON
as text and `structuredContent`, set `isError` on refusals, return PNG outputs as image content, and record edits with
source `mcp`.

#### Scenario: Edit through MCP
- **WHEN** a client calls `print_timeline`, edits the text and calls `apply_timeline` with it
- **THEN** the project has a new revision with source `mcp`

### Requirement: Media and timeline inspection
The CLI SHALL decode frames and contact sheets from media, find hard cuts, report audio levels, silent ranges and
loudness (ITU-R BS.1770 / EBU R128), and render composited timeline frames identical to export.

#### Scenario: Loudness
- **WHEN** `ove audio loudness` measures a mono 1 kHz sine peaking at −38.06 dBFS
- **THEN** the integrated loudness is −41.07 LUFS within 0.3 LU
