# Design: add-local-agents-and-onboarding

## D1. The short list

| Agent | Vendor | Started as | Sign-in it uses |
| --- | --- | --- | --- |
| Claude | Anthropic | `claude` through the Agent Client Protocol adapter | the person's Claude login or key |
| Codex | OpenAI | `codex` through the Agent Client Protocol adapter | the person's ChatGPT login or key |
| OpenCode | Anomaly | `opencode acp` | the model the person configured |
| Hermes | Nous Research | `hermes acp` | the model the person configured |
| Kimi | Moonshot AI | `kimi acp` | the person's Kimi login or key |
| Grok | xAI | `grok agent stdio` | the person's Grok login or key |

The list is data (`ove-agents`: id, display name, vendor, executable names, version flag, how it is started, how it
is given an MCP server), so adding an agent is one entry and its tests, not new code paths.

Naming follows the vendors' rules: the Claude entry is labelled "Claude", never "Claude Code".

## D2. Detection

At start-up and when Settings → Assistant opens, off the main thread:

1. For each entry, look for its executable on `PATH` and in the install folders its installer uses (`~/.local/bin`,
   `~/.claude/local`, npm and pnpm global bins, Homebrew's bin on macOS, `%APPDATA%\npm` and `%LOCALAPPDATA%\Programs`
   on Windows). The first match wins.
2. Run `<executable> --version` with no window, a 5 s limit and stdin closed; keep the first line, at most 80
   characters.
3. Report one of: **Ready** (found, version read), **Found** (found, version not read), **Not installed**.

Detection never reads an agent's credential files. Whether the agent is signed in is learned the first time a session
is started: a start that fails for sign-in shows the agent's own instruction ("Run `claude` in a terminal and sign
in"), and the agent is marked **Needs sign-in** until a session succeeds.

## D3. Icons

Each agent is shown with its own logo on a neutral tile (`ui/src/agents/logos.tsx`, generated from LobeHub's icon
set, MIT). The licences that ship with the app name the trademark owners and say that showing a logo is not an
endorsement. An entry without a logo (the built-in assistant on an API key, or an agent whose logo is removed) gets a
lettered tile drawn for this app. The app's own name and icon are always more prominent than any agent's.

## D4. Connect (agent in its own terminal)

Connect gives an installed agent the app's MCP server and editing guide:

- the editing guide: what `ove skills install` does today, for that agent's skills folder;
- the MCP server: the agent's own command, run by the app (`claude mcp add openvideoeditor -- <ove> mcp`,
  `codex mcp add openvideoeditor -- <ove> mcp`).

An agent whose command has not been checked by us against a real install shows the command to copy instead of a
button. Today that leaves one button, Claude's (its command was read from the installed program's own help); Codex's
command comes from OpenAI's documentation and the other four are set up in the agent's own settings.
The app never edits an agent's configuration files itself. Connect is idempotent and reports what the agent printed.

## D5. In-app chat through a local agent

```
chat panel ──► assistant backend ──┬─► built-in (Anthropic Messages API, the person's key)   [today]
                                   └─► local agent (Agent Client Protocol over stdio)         [this change]
                                              │  session/new: cwd = an empty session folder,
                                              │               mcpServers = [ove mcp, pinned to the open project]
                                              ▼
                                         the agent ──► ove mcp ──► control link ──► the open project
```

- The backend is a trait with the two implementations; the chat panel, turn summaries, Show and Undo this turn use
  the same events for both.
- The session's MCP server is `ove mcp` started with the open project pinned and the turn's id, so every edit is a
  revision attributed to the turn.
- **The agent sees the project.** The pinned server offers, besides the editing tools: a frame of the composited
  timeline or of a media file at a time, a contact sheet of the timeline, frame statistics, sound levels, silences
  and loudness, transcripts and captions, and the media list with what each file is. Pictures are returned as images
  in the tool result, scaled to at most 1280 pixels on the long side. The built-in assistant gets the same tools, so
  both back ends see the same things.
- Subscription agents do not report a cost in money: such a turn shows "uses your <agent> plan" where the built-in
  assistant shows a price. Spend caps apply to the built-in assistant only.

## D6. The harmless agent

Defence in depth; each layer holds without the others.

1. **What the agent is offered.** The session is started with the agent's own switches for a tool allow-list holding
   only the app's MCP tools, its shell and file tools denied, and its most restrictive permission mode.
2. **What the app allows.** The app is the Agent Client Protocol client: every permission request for anything other
   than one of the app's MCP tools is refused, and the file-system and terminal capabilities of the protocol are not
   offered at all.
3. **What the tools can do.** The in-app MCP server exposes editing tools and the tools for seeing the project (D5),
   pinned to one project. Seeing is reading through the app: a frame or a measurement of media that is in the open
   project, never a file by its path. No tool takes a path outside the project, deletes media, or writes a file;
   import and export are asked of the person.
4. **Where it runs.** The agent's working folder is an empty folder made for the session.
5. **What can be undone.** Every change is a revision; Undo this turn and History restore it.
6. **Limits.** Steps and time per turn are capped as for the built-in assistant, and Stop ends the agent's process.

Text that arrives from media (file names, captions, metadata) is data. With no shell, file or network tool on offer,
the most such text can cause is a timeline edit, which is a revision.

A stand-in agent that tries a shell command, a file write, a file read outside the session folder, a network request
and a second project is part of the test suite on every platform; each attempt must be refused and logged.

## D7. Onboarding

- **State** is kept by the native side in the app's configuration folder (`onboarding.json`: version, finished or
  skipped, checklist steps done), not in the web profile, so it survives a cleared profile and is the same for every
  window.
- **Welcome**: at most three screens (what the app does; pick an assistant; privacy and where things are kept), each
  with Skip. Picking nothing is "I'll edit by hand". The privacy screen says: media files stay on this computer; the
  assistant you chose sees the timeline, frames of your video, sound measurements and transcripts, and those go to
  that assistant's provider; nothing is sent when no assistant is chosen.
- **Sample project**: a project made from media shipped inside the app (a few seconds, made for the app, a few hundred
  kilobytes), with a cut, a title, captions and a transition already in it. It is copied into the library on request;
  the shipped files are never edited.
- **Guided first edit**: five steps (Import, Cut, Title, Captions, Export). A step is ticked when the project shows it
  was done, not when a button is pressed. The list can be closed and reopened from Help.
- **Hints**: every empty panel says what it is for and offers one action. **Starter prompts**: three one-tap examples
  in an empty chat and an answer to "What can you do?".
- **Help**: one entry on Home and in the editor that reopens the welcome, the checklist and the sample.

The app has no telemetry, so onboarding is judged by watching people use it, not by counting.

## D8. Order

1. Detection, the picker with icons, Connect for Claude and Codex; onboarding (welcome, sample, checklist, Help).
2. In-app chat through a local agent with the harmless-agent rules and the hostile stand-in; hints and starter
   prompts that depend on the chosen assistant.
3. Vendor artwork where permitted; Connect for the remaining agents as each is checked against a real install.
