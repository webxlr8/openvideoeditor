# `ove skills`

Skills are playbooks that agents read: how to style captions, how to match colour, how to cut an interview. Each
skill is one Markdown file. Every agent surface can read them: the in-app assistant, `ove mcp`, and other agents you
export them to.

```sh
ove skills list [--json]
ove skills show <id> [--json]
ove skills install [--dir <skills folder>]...
```

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

## Examples

```sh
ove skills list                    # exit 0
ove skills show caption-styling    # exit 0
ove skills show no-such-skill      # exit 1
```

## The editing guide

`ove skills install` with no id writes the `openvideoeditor` editing guide (`ove docs agents` with front matter) into
the Claude Code and Codex skills folders that exist (`~/.claude/skills`, `~/.codex/skills`), or into each `--dir`.
