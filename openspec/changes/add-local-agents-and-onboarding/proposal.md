# Proposal: add-local-agents-and-onboarding

## Why

Two things stop a new person from getting value out of the app:

- The assistant asks for an Anthropic API key before it does anything. Most people who would use an assistant already
  have an AI agent installed and signed in on their computer (Claude, Codex, OpenCode, Hermes, Kimi, Grok). The app
  should use that agent, with the sign-in it already has, and ask for nothing.
- The app opens on an empty Home with no guidance. Nothing shows a new person what the app does or how to make a
  first video.

## What Changes

- **Local agents.** The app finds the supported agents installed on the computer and offers them, each with an icon,
  wherever an assistant is chosen. An API key becomes a fallback, not a requirement.
- **A short list, not every agent.** Six agents are supported, chosen for use and for a documented way to be driven by
  another program (the Agent Client Protocol): Claude, Codex, OpenCode, Hermes, Kimi, Grok.
- **One-click Connect.** An agent used from its own terminal is given the app's editing tools with one click instead
  of copied commands.
- **In-app chat through a local agent.** The chat panel talks to the chosen agent over the Agent Client Protocol; the
  agent edits through the app's existing editing tools, so revisions, Show and Undo this turn work as today.
- **Agent first: the agent sees the project.** The agent gets what an editor at the screen has: the timeline, frames
  of the picture (single frames and contact sheets), sound levels, silences and loudness, transcripts, and what each
  media file is. This is on from the start; the welcome says plainly that frames and sound measurements go to the
  agent the person chose.
- **A harmless agent.** Inside the app an agent can only use the app's tools on the open project: no shell, no file
  writes, no network, no other project. Every change is a revision that can be undone. A hostile stand-in agent is run
  on every build and everything it tries must be refused.
- **Self-onboarding.** A first run of at most three screens (what the app does, pick an assistant, privacy), a sample
  project so the editor is never empty, a guided first edit that ticks off as the person does each step, hints in
  empty panels, starter prompts in the chat, and a Help entry that reopens all of it.

## Out of scope

- Agents outside the short list. Gemini CLI was closed to consumer plans in June 2026 and its replacement has no
  documented Agent Client Protocol mode; GitHub Copilot's is in preview; Cursor follows with it. They are candidates
  for a later change.
- Generation (images, video, voice, music) without provider keys: coding agents do not generate media. The app asks
  for a provider key at the first use of generation only.
- Offering a vendor's sign-in inside the app. The app never shows a sign-in for an agent and never reads, stores or
  forwards an agent's credentials; it starts the agent the person installed, as that person.

## Decisions that are the owner's

- **Advertising "use your Claude subscription".** Anthropic's Agent SDK documentation says third parties may not offer
  claude.ai login or rate limits in their products without approval, and its help centre says third-party apps draw
  from a signed-in person's plan. Starting the person's own installed agent sits between the two. The agent is
  supported; the wording in marketing waits for Anthropic's answer.
- **Vendor logos.** OpenAI's brand guidelines allow its marks only under its guidelines and require approval for
  co-branding; the other vendors' terms are unchecked. The app ships neutral icons of its own in a slot that takes the
  vendor's artwork once its terms are confirmed.

## Decided

- **Frames and everything to the agent (owner, 14 November 2026).** The product is agent first. The agent sees frames,
  sound measurements and transcripts of the open project by default. The privacy text in the app and in the release
  notes changes with it: media files stay on the computer, and what the agent looks at goes to the agent's provider.
