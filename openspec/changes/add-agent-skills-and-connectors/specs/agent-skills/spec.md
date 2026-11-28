# Spec Delta

## Purpose

Skills are text playbooks that agents read. They are kept in one store, shown to every agent surface, installed and
updated from a catalog with hash checks, exported to other local agents, and drafted from finished projects.

## ADDED Requirements

### Requirement: Skill store
Each skill SHALL be stored once in the library as a folder that contains a skill file. The skill file SHALL start with a
name and a description. A skill with a missing or invalid name or description SHALL be listed as invalid. Agents SHALL
NOT be offered an invalid skill.

#### Scenario: Valid skill listed
- **WHEN** a folder with a valid skill file is added to the store
- **THEN** the skill appears in Settings → Skills and in `list_skills`

#### Scenario: Invalid skill
- **WHEN** a skill file has no description
- **THEN** Settings → Skills marks it invalid with the reason, and `list_skills` does not return it

### Requirement: Skill index and reading skills
The in-app assistant SHALL receive a one-line index of the installed skills with each turn. Every agent surface SHALL
offer `list_skills` and `read_skill`. Adding or removing a skill SHALL NOT change the fixed part of the in-app system
prompt.

#### Scenario: Agent reads a skill
- **WHEN** the assistant calls `read_skill` with the id of an installed skill
- **THEN** it receives the full skill text

#### Scenario: Unknown skill
- **WHEN** an agent calls `read_skill` with an id that is not installed
- **THEN** the call fails with `skill.unknown`, and the response lists the installed ids

#### Scenario: Cache kept
- **WHEN** a skill is installed between two assistant turns
- **THEN** the fixed system prompt text sent in both turns is byte-identical

### Requirement: Skills as MCP prompts and resources
The MCP server SHALL list each installed skill as a prompt and as a resource. Requesting either SHALL return the skill
text.

#### Scenario: Prompt listed
- **WHEN** an MCP client requests the prompt list
- **THEN** each installed valid skill appears with its name and description

### Requirement: In-app skill management
The in-app assistant SHALL be able to create, update and remove skills that are not builtin. Each such change SHALL
appear in Settings → Skills with its source. Builtin skills SHALL NOT be changed or removed by an agent.

#### Scenario: Agent creates a skill
- **WHEN** the person asks the assistant to save this workflow as a skill
- **THEN** a new skill appears in Settings → Skills marked as created by the assistant

#### Scenario: Builtin protected
- **WHEN** an agent tries to remove a builtin skill
- **THEN** the call fails with `skill.builtin`, and the skill remains

### Requirement: Catalog install with hash check
Installing from the configured catalog SHALL fetch the skill file and compare it with the full hash in the catalog
entry. The skill SHALL be installed only when the hashes match.

#### Scenario: Install
- **WHEN** the person installs a catalog skill whose content matches its hash
- **THEN** the skill is installed, and its source is shown as that catalog

#### Scenario: Tampered content
- **WHEN** the fetched content does not match the catalog hash
- **THEN** nothing is installed, and the person sees that the content failed verification

#### Scenario: No catalog configured
- **WHEN** no catalog address is set
- **THEN** the Community section explains how to add one, and no network request is made

### Requirement: Visible updates that never overwrite edits
A changed catalog entry SHALL be offered as an update with a view of the differences. It SHALL be installed only after
the person accepts it. A skill that the person modified locally SHALL NOT be overwritten. Its update SHALL be offered as
a separate copy.

#### Scenario: Accept update
- **WHEN** the catalog entry of an unmodified installed skill changes and the person accepts the update
- **THEN** the new content replaces the old one

#### Scenario: Local edits kept
- **WHEN** the person edited an installed skill and the catalog entry changes
- **THEN** the edited skill is unchanged, and the update is offered as a new copy with a different id

### Requirement: Export to other agents
The person SHALL be able to export a skill into the skill folder of each detected local agent that supports skills. An
exported skill SHALL get a name prefix. A copy that the person changed in the other agent's folder SHALL NOT be
overwritten without confirmation.

#### Scenario: Export to Claude Code
- **WHEN** the person exports a skill to Claude Code
- **THEN** the skill appears in Claude Code's skill folder with the prefix, and Claude Code can list it

#### Scenario: Agent not installed
- **WHEN** the person opens the export menu and Cursor is not detected
- **THEN** Cursor is shown as unavailable, with the reason

### Requirement: Draft a skill from a project
The person SHALL be able to draft a skill from a project, from the CLI and in the app. The draft SHALL be built from:
- the final timeline;
- the edit history by actor;
- the presets used;
- the chat, when the person chooses to include it.

The draft SHALL open for review and SHALL be saved only when the person saves it.

#### Scenario: Draft reviewed
- **WHEN** the person drafts a skill from a finished project
- **THEN** an editor opens with a draft that lists the steps and values used, and no skill is installed until the
  person saves it

#### Scenario: No assistant configured
- **WHEN** no assistant is configured
- **THEN** drafting stops and explains that an assistant is needed, and the context pack can still be saved to a file

### Requirement: Starter skills
The app SHALL ship 3 builtin skills: caption styling, colour matching and interview cutting. They SHALL be listed as
builtin.

#### Scenario: Fresh install
- **WHEN** the app starts for the first time
- **THEN** Settings → Skills lists the 3 builtin skills, and `list_skills` returns them
