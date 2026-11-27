# `ove models`

Local machine-learning models (speech recognition today; voice activity, beat detection, visual search and
subject mattes as those features arrive). Models are never in the installer: each downloads on first use into the
model store, which every project shares. The app shows the same list in Settings → Storage.

```sh
ove models list [--json]
ove models install <id> [--yes] [--json]
ove models remove <id> [--json]
```

## The store

The store is the folder `models` in the app's data folder:

- Windows: `%APPDATA%\dev.openvideoeditor.app\models`
- macOS: `~/Library/Application Support/dev.openvideoeditor.app/models`

`OVE_MODELS_DIR` overrides it (the former `OVE_WHISPER_MODEL_DIR` still works). Inside it, each model has the folder
`<id>/<version>/` with its files, and `installed.json` records what is installed: the id, version, each file's path,
size and SHA-256, and when it was installed.

A model counts as installed only when `installed.json` records it at the current version and every file is on disk
with its pinned size. Speech models saved by earlier versions directly in the store folder (`ggml-base.bin` and so
on) are checked once and recorded where they are, so they never download again.

## Manifests

Each model is described by a manifest compiled into `ove`: an id, a version, its purpose, the licence of its
weights, whether it may run inside the editor (10 MB or less), and its files. Every file is pinned by size and
SHA-256 and comes from a fixed location: a commit or a versioned release, never a moving branch. Downloads go to
`<file>.part`, are hashed while they stream, and are renamed only when the hash matches; a damaged file is deleted.
An interrupted download continues from the bytes already received the next time. Before a download starts, the store
checks that the free space is at least the bytes still to download plus 10 % of the model.

`OVE_MODEL_BASE_URL` makes downloads come from a mirror (`<base>/<id>/<version>/<file>`) instead of the pinned
locations; the pinned sizes and hashes still apply.

## Consent

A model larger than 50 MB downloads only after the person agrees: in the app's prompt, or with `--yes` typed at the
terminal. Without `--yes`, and always over MCP, `install` refuses with exit 1 and code `model.consent`; the details
carry the model's `id`, `purpose`, `bytes` and the `install` command. Models of 50 MB or less download without a
prompt.

## Commands

`list` prints every model with its size, whether it is installed, its purpose and, when installed, its folder. With
`--json`: `{"root", "installedBytes", "models": [{"id", "version", "purpose", "licence", "runtime", "inProcess",
"bytes", "installed", "location"}]}`.

`install` downloads and verifies a model (progress on stderr) and prints `installed <id>`, or `<id> is already
installed` without downloading anything. With `--json`: `{"id", "installed", "already", "bytes", "paths"}`.

`remove` deletes an installed model's files and prints the space freed and the free space left. With `--json`:
`{"id", "removed", "freedBytes", "freeBytes"}`.

Exit codes: 0 done, 1 refused. Refusal codes: `model.unknown` (no such model), `model.consent` (over 50 MB without
consent), `model.notInstalled` (remove of a model that is not installed), `model.noSpace`, `model.corrupt`,
`model.download`, `model.busy` (the model is already downloading).

## Examples

With an empty store:

```sh
ove models list --json                 # exit 0
ove models install whisper-base        # exit 1
ove models install whisper-base --json # exit 1
ove models install no-such-model       # exit 1
ove models remove whisper-tiny         # exit 1
```

The second line refuses because the speech model is a 148 MB download: run `ove models install whisper-base --yes`
to download it.
