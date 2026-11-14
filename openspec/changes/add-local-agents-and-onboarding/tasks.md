# Tasks: add-local-agents-and-onboarding

## 1. Agents: the short list, detection, the picker

- [ ] 1.1 `ove-agents` crate: the six entries as data (id, name, vendor, executables, version flag, start command,
      MCP-add command and whether it is checked).
- [ ] 1.2 Detection: search path and install folders, `--version` with a 5 s limit and no window, Ready / Found / Not
      installed. Tests with stand-in executables: installed, missing, hangs, prints rubbish.
- [ ] 1.3 Tauri command and bindings; detection off the main thread; `ove doctor` lists the agents.
- [ ] 1.4 Picker with icons (own neutral icons in the slot) in Settings → Assistant and the chat panel; no key form
      when an agent is Ready.
- [ ] 1.5 Connect for Claude and Codex (editing guide + the agent's own `mcp add`), idempotent; copyable command for
      the other four. Tests with stand-in executables that record what they were asked.

## 2. Onboarding

- [ ] 2.1 Onboarding state in the configuration folder (native), commands to read and update it.
- [ ] 2.2 Welcome: three screens, Skip on each, the assistant picker from 1.4.
- [ ] 2.3 Sample project: media made for the app and shipped as a resource; "Try the sample" copies it into the
      library with a cut, a title, captions and a transition.
- [ ] 2.4 Guided first edit: five steps ticked from the project's state; close and reopen.
- [ ] 2.5 Help entry on Home and in the editor: Getting started, sample project, shortcuts.
- [ ] 2.6 Hints in empty panels; starter prompts in an empty chat.
- [ ] 2.7 App end-to-end: a first-run flow on a clean profile (Linux, Windows); the other flows dismiss the welcome.

## 3. Chat through a local agent

- [ ] 3.1 Assistant backend trait; the built-in assistant behind it unchanged.
- [ ] 3.2 Agent Client Protocol client over stdio: initialise, new session with the pinned MCP server, prompt, stream,
      permission requests, cancel.
- [ ] 3.3 `ove mcp` pinned to one project and one turn (revisions attributed to the turn; other projects refused).
- [ ] 3.3a Tools for seeing the project in the pinned server and for the built-in assistant: timeline and media
      frames, contact sheet, frame statistics, sound levels, silences, loudness, transcripts; pictures as images, at
      most 1280 pixels; media outside the open project refused.
- [ ] 3.4 Harmless-agent rules (D6 layers 1 to 4 and 6) and the hostile stand-in agent in the suite on every platform.
- [ ] 3.5 Sign-in needed: the agent's own instruction in the chat; "uses your plan" where a price would be.
- [ ] 3.6 A scripted stand-in agent for the app end-to-end suite: a turn edits the timeline, Show, Undo this turn.
- [ ] 3.7 Each of the six agents run once by hand against a real install; results recorded here.

## 4. After

- [ ] 4.1 Vendor artwork in the icon slots where the vendor's terms allow it.
- [ ] 4.2 Connect buttons for OpenCode, Hermes, Kimi and Grok as each command is checked against a real install.
- [ ] 4.3 Release notes and the in-app "What's new", with the changed privacy text (the assistant sees frames and
      sound measurements; media files stay on the computer).
