# Spec Delta

## Purpose

The assistant runs on an AI agent the person already has installed and signed in, and that agent can change nothing
but the open project.

## ADDED Requirements

### Requirement: Supported agents
The app SHALL support exactly the agents on its short list (Claude, Codex, OpenCode, Hermes, Kimi, Grok), each
described by one data entry, and SHALL label each by the name its vendor allows.

#### Scenario: Supported agents
- **WHEN** the assistant picker is shown
- **THEN** it lists the six agents and "API key", and the Claude entry reads "Claude", never "Claude Code"

### Requirement: Detection
The app SHALL find each supported agent's executable on the search path and in its usual install folders, read its
version with a time limit, and report it as Ready, Found or Not installed, without reading any credential file.

#### Scenario: Installed agent
- **WHEN** an agent's executable is on the search path and answers `--version`
- **THEN** the agent is shown as Ready with that version

#### Scenario: Missing agent
- **WHEN** an agent's executable is nowhere the app looks
- **THEN** the agent is shown as Not installed, with where to get it

#### Scenario: Agent that hangs
- **WHEN** an agent's executable does not answer `--version` within the limit
- **THEN** it is stopped, shown as Found, and the app stays responsive

### Requirement: No key when an agent is ready
The app SHALL NOT ask for an API key in the chat panel or during onboarding when a supported agent is Ready; a key
SHALL remain available as a choice.

#### Scenario: No key when an agent is ready
- **WHEN** one agent is Ready and no key is saved
- **THEN** the chat panel offers that agent and does not show the key form

### Requirement: Icons
Every agent SHALL be shown with an icon. The app SHALL ship icons of its own and SHALL use a vendor's artwork only
from the agent entry's icon slot.

#### Scenario: Icons
- **WHEN** the picker is shown before any vendor artwork is added
- **THEN** every agent has a distinct icon drawn for this app

### Requirement: Connect
For an agent whose command has been checked against a real install, Connect SHALL install the editing guide and
register the app's MCP server by running the agent's own command, and report the outcome. For any other agent the
app SHALL show the command to copy. The app SHALL NOT edit an agent's configuration files itself.

#### Scenario: Connect
- **WHEN** Connect is pressed for a Ready agent with a checked command
- **THEN** the agent's own command is run once and the picker shows it as connected

#### Scenario: Connect twice
- **WHEN** Connect is pressed for an agent that is already connected
- **THEN** nothing is duplicated and the outcome is reported as already connected

### Requirement: Chat through a local agent
The chat panel SHALL be able to run a turn through the chosen local agent over the Agent Client Protocol, with the
app's MCP server pinned to the open project as the agent's only tools.

#### Scenario: A turn through a local agent
- **WHEN** a turn asks for a title and a local agent is chosen
- **THEN** the title appears on the timeline, the turn shows what changed, and Undo this turn removes it

#### Scenario: Agent not signed in
- **WHEN** the chosen agent refuses to start for lack of a sign-in
- **THEN** the chat shows that agent's own sign-in instruction and nothing else changes

### Requirement: The agent sees the project
An agent SHALL be able to get frames of the timeline and of the project's media, contact sheets, frame statistics,
sound levels, silences and loudness, transcripts, and the media list, through the app's tools, from the first turn
and without a setting to turn on. Pictures SHALL be returned as images in the tool result.

#### Scenario: A frame
- **WHEN** an agent asks for the frame at 2 seconds of the timeline
- **THEN** it receives that frame as an image no larger than 1280 pixels on its long side

#### Scenario: Only the open project
- **WHEN** an agent asks for a frame of a file by a path that is not media of the open project
- **THEN** the request is refused

### Requirement: What the agent sees is stated
The welcome and Settings → Privacy SHALL state that the chosen assistant sees the timeline, frames, sound
measurements and transcripts, that these go to that assistant's provider, and that media files stay on the computer.

#### Scenario: What the agent sees is stated
- **WHEN** the privacy screen of the welcome is shown
- **THEN** it names frames and sound measurements among what the assistant sees

### Requirement: Harmless agent
Inside the app an agent SHALL be able to use only the app's tools (editing and seeing) on the open project. The app SHALL refuse
every other request (shell, file read or write, network, another project), SHALL NOT offer the protocol's file-system
or terminal capabilities, and SHALL run the agent in an empty session folder.

#### Scenario: Shell command
- **WHEN** an agent asks to run a shell command
- **THEN** the request is refused, nothing runs, and the refusal is logged

#### Scenario: File outside the session
- **WHEN** an agent asks to read or write a file
- **THEN** the request is refused

#### Scenario: Another project
- **WHEN** an agent's tool call names a project other than the open one
- **THEN** the call is refused

### Requirement: Everything an agent does can be undone
Every change an agent makes SHALL be a revision attributed to its turn, and Stop SHALL end the agent's process.

#### Scenario: Undo
- **WHEN** an agent's turn changed three items
- **THEN** Undo this turn restores all three in one step

#### Scenario: Stop
- **WHEN** Stop is pressed during a turn
- **THEN** the agent's process ends and changes already made remain as revisions

### Requirement: Credentials stay with the agent
The app SHALL NOT show a sign-in for an agent, and SHALL NOT read, store or forward an agent's credentials.

#### Scenario: Credentials stay with the agent
- **WHEN** a local agent is used for a turn
- **THEN** no file or log of the app contains a credential of that agent
