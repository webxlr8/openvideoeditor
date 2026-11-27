# `ove skills`

Skills are playbooks that agents read: how to style captions, how to match colour, how to cut an interview. Each
skill is one Markdown file. Every agent surface can read them: the in-app assistant, `ove mcp`, and other agents you
export them to.

```sh
ove skills list [--json]
ove skills show <id> [--json]
ove skills add <file> [--id <id>] [--json]
ove skills catalog [--set <https address> | --clear] [--json]
ove skills install <id> [--as <id>] [--yes --sha256 <pin>] [--json]
ove skills update [<id>] [--yes] [--json]
ove skills export <id> [--to claude,codex,cursor] [--yes] [--json]
ove skills draft --from-project <project> [--with-chat] [--out <file>] [--pack-out <file>] [--model <id>] [--json]
ove skills install [--dir <skills folder>]...
```

Settings → Skills in the app does the same: Installed (builtin and catalog skills), Community (the catalog) and Local
(your own skills, the ones the assistant saved, and drafts), an editor, and an export menu on each skill.

## The store

Skills live in the library, in the folder `.skills`:

```
<library>/.skills/
  ledger.json
  <id>/SKILL.md
```

An id is 1 to 64 characters of `a-z`, `0-9` and `-`. `SKILL.md` starts with front matter that gives a name and a
description:

```markdown
---
name: Tidy cuts
description: Close small gaps on the main track. Use when the person asks to tighten the edit.
version: 1
---

The steps the agent follows.
```

- `name` is one line of at most 80 characters, and `description` at most 1,024 characters. Both are required.
- `version` is optional. Other keys are ignored and never take effect.
- The file is at most 256 KB of UTF-8 text, with some text after the front matter.

A skill that breaks a rule is listed as invalid with the reason, and agents are not offered it. Folders that are links
are skipped.

`ledger.json` records, for each skill the app installed or saved, the SHA-256 of the file and where it came from. A
file whose hash differs from its ledger entry is shown as `modified`.

## Builtin skills

The app ships 3 skills, written for it: `caption-styling`, `colour-matching` and `interview-cutting`. They are listed
first, with the source `builtin`. Only an app release changes them, and no agent can change or remove them. A folder in
the store with a builtin id is listed as invalid.

## How agents read skills

- The in-app assistant gets a one-line index of the valid skills with each turn: the id, then the description cut at
  200 characters, for at most 60 skills. The index is a separate system block after the fixed prompt, so adding a
  skill does not change the fixed prompt, and that part of the prompt stays cached.
- Every agent surface has the tools `list_skills` and `read_skill`. `read_skill` with an id that is not installed is
  refused with `skill.unknown`; its details list the installed ids. An invalid skill is refused with `skill.invalid`.
- `ove mcp` lists each valid skill as a prompt (`prompts/list`, `prompts/get` returns the skill as one user message)
  and as the resource `ove://skills/<id>` (`resources/list`, `resources/read`).

Skills are text only. Nothing in a skill runs, and a skill cannot widen which tools an agent may use. The text an
agent reads starts with a line that names the skill's source.

## Skills the assistant saves

The in-app assistant has the tool `manage_skills` (create, update, remove). Use it by asking, for example, "save this
way of working as a skill". Its skills show in Settings → Skills with the source "created by the assistant". It
cannot change or remove builtin skills (`skill.builtin`). Agents outside the app cannot change skills.

## The community catalog

A catalog is a JSON document at an HTTPS address:

```json
{"skills": [{"id": "pacing", "name": "Pacing", "description": "Keep a steady pace.", "sha256": "<64 hex digits>", "path": "skills/pacing.md"}]}
```

- No catalog is set by default, and with none set nothing is fetched. Set one with `ove skills catalog --set <url>`
  or in Settings → Skills → Community. `OVE_SKILLS_CATALOG` overrides the setting.
- Only `https://` addresses are accepted (and `http://` to this computer, for testing). A redirect may not leave
  HTTPS. `path` is relative to the catalog document and must stay in its folder.
- Each entry pins the full SHA-256 of its file. A file that does not match is refused with `skill.tampered`, and
  nothing is written.
- `ove skills install <id>` shows the verified file and its pin and installs nothing. `--yes --sha256 <pin>` installs
  it; the pin must be the one shown with the text you read, so a catalog cannot show one text and install another
  (`skill.changed`). Settings installs the text it showed you the same way. The skill's source is
  shown as `catalog <address>`. An id that is taken is refused with `skill.exists`; use `--as <id>`.

Catalog skills are text from someone else. They are checked like any skill, their front matter cannot set their own
source, and they cannot change which tools an agent may use.

### Updates

`ove skills update` lists the skills installed from the catalog with their state: `upToDate`, `available`, `modified`
(you edited it and the catalog changed), `editedLocally` or `gone`. `ove skills update <id>` shows the differences
and changes nothing; with `--yes` it installs the update:

- An unmodified skill is replaced.
- A skill you edited is never overwritten. The update is installed as a copy, `<id>-update`.

An update is never installed without your confirmation.

## Export to other agents

`ove skills export <id>` copies a skill into the skills folder of each agent installed for this user: Claude
(`~/.claude/skills`), Codex (`~/.codex/skills`) and Cursor (`~/.cursor/skills`). An agent counts as installed when
its folder (`~/.claude`, `~/.codex`, `~/.cursor`) exists; the others are named with the reason
(`export.unavailable`).

- The copy is `ove-<id>/SKILL.md`. Its front matter is written fresh: `name: ove-<id>` and the description. No other
  key from the skill file is copied, so a skill cannot change what the other agent allows.
- If you changed the copy in the agent's folder since the last export, it is not replaced (`export.modified`) unless
  you pass `--yes` (Replace it in Settings).

## Draft a skill from a project

`ove skills draft --from-project <project>` asks the assistant (your Anthropic API key from Settings → Assistant, or
`ANTHROPIC_API_KEY`) to write a skill from a finished project. The request holds a context pack:

- the final `timeline.tsx`, with its media table;
- one line per revision, grouped by who made it;
- the caption styles, transitions, looks and other presets in use;
- with `--with-chat`, the project's latest chat with the assistant.

The draft answers two questions: how to rebuild this edit from the same media, and how to apply its style to new
media. Nothing is saved: read the draft, change what you need, then save it with `ove skills add <file>` (or Save in
Settings → Skills → Local). Without an assistant, drafting stops with `draft.noAssistant`; `--pack-out <file>` still
saves the context pack.

## Examples

```sh
ove skills list                    # exit 0
ove skills show caption-styling    # exit 0
ove skills show no-such-skill      # exit 1
ove skills catalog                 # exit 1
ove skills install pacing --yes    # exit 1
```

With no catalog set, the last two are refused with `catalog.unset` and nothing is fetched.

## The editing guide

`ove skills install` with no id writes the `openvideoeditor` editing guide (`ove docs agents` with front matter) into
the Claude Code and Codex skills folders that exist (`~/.claude/skills`, `~/.codex/skills`), or into each `--dir`.
