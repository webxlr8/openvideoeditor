# `ove models`

Local machine-learning models (speech recognition today; voice activity, beat detection, visual search and
subject mattes as those features arrive). Models are never in the installer: each downloads on first use into the
model store, which every project shares. The app shows the same list in Settings → Storage.

```sh
ove models list [--json]
ove models install <id> [--yes] [--from <folder>] [--json]
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

A model larger than 50 MB, and the inference runtime whatever its size, downloads only after the person agrees: with
Install in Settings → Storage, or with `--yes` typed at the terminal. Without `--yes`, and always over MCP, `install`
refuses with exit 1 and code `model.consent`; the details carry the model's `id`, `purpose`, `bytes` (installed size),
`downloadBytes`, the `host` it downloads from, the `licence` of its weights and the `install` command. Other models
of 50 MB or less download without a prompt. The first progress line of a download names the size, the licence and
the host (`downloading <id> (<size>, licence <licence>) from <host>`). Sizes from 500 MB up read in gigabytes
(`0.63 GB`), smaller ones in megabytes.

## Visual search

The media search model `siglip2-base` (SigLIP 2 base, patch 16, 256 px; licence Apache-2.0) is a 0.63 GB download
from `huggingface.co`: the image encoder in half precision (186 MB), the text encoder with 4-bit weights (443 MB)
and the tokenizer (4 MB). It needs consent like any model over 50 MB, and the inference runtime. Releasing it is on
hold until the owner signs off its licence review (see `openspec/changes/add-semantic-media-search/design.md`).
See [media-search.md](media-search.md).

## The inference runtime

Voice activity detection and the other ONNX models run on ONNX Runtime 1.32.0 (MIT, by Microsoft), which is not in the
installer. It installs into the store as the model `onnxruntime`:

- It comes from the official Python wheel on the Python package index (`files.pythonhosted.org`), whose file
  locations are content addressed. The wheel is checked against its pinned size and SHA-256, the library is extracted
  and checked against its own pins, and the wheel is deleted. Download: 16.7 MB on Windows (library 22.7 MB), 22.5 MB
  on macOS (library 35.3 MB).
- The wheel's `LICENSE` and `ThirdPartyNotices.txt` are installed next to the library in
  `onnxruntime/1.32.0/`.
- macOS needs version 14 or later on Apple silicon (the wheel is built for `macosx_14_0_arm64`).
- A feature that needs the runtime when it is not installed reports `runtime not installed` with the size, the host
  and the install command, and does not download it by itself.
- `OVE_ONNXRUNTIME_LIB` names a library file to use instead of the store's.

The pinned wheels:

| Platform | Wheel | Bytes | SHA-256 |
|---|---|---|---|
| Windows x64 | `onnxruntime-1.32.0-cp312-cp312-win_amd64.whl` | 16,698,767 | `efe7456952c243a05a53d5b8732953625144642c2f3989094277f6217f0c0c40` |
| macOS arm64 | `onnxruntime-1.32.0-cp312-cp312-macosx_14_0_arm64.whl` | 22,519,308 | `d52510bbbcc880c13b826f7d75cdf33ac5cf0bb864d960000fc29ce73f0cb83e` |

### Offline install

On a computer without internet access (or behind a proxy that blocks the package index):

1. On a computer with access, download the wheel for the target platform from the table above (it is on the
   `onnxruntime` 1.32.0 page of the Python package index) and check its SHA-256.
2. Copy it, under its original file name, into a folder on the target computer.
3. Run `ove models install onnxruntime --from <folder>`.

`--from` takes any model: the folder must hold each file under the name at the end of its pinned URL (the model file,
or the archive it comes from). Every size and hash is checked as for a download, nothing is fetched, and the files in
the folder are left in place. `--from` works only from the terminal, not over MCP.

## Commands

`list` prints every model with its size, whether it is installed, its purpose and, when installed, its folder. With
`--json`: `{"root", "installedBytes", "models": [{"id", "version", "purpose", "licence", "runtime", "inProcess",
"bytes", "installed", "location", "host", "downloadBytes"}]}`.

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
ove models install onnxruntime         # exit 1
ove models install onnxruntime --from no-such-folder # exit 1
```

The second line refuses because the speech model is a 148 MB download: run `ove models install whisper-base --yes`
to download it. The sixth refuses because the inference runtime always needs consent; the seventh because the folder
does not hold the wheel.
