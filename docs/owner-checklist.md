# Owner checklist

These checks need the owner's machines, accounts or approval. Everything that can run on the Windows development
machine is not listed here; it is done there. Each row names the change and task. The task text in
`openspec/changes/<change>/tasks.md` is the source of truth, so this list adds no acceptance criteria of its own.
Record each measured number in that change's `design.md` under Results, tick the task, and delete the row here.

The rows are in order of what they unblock. Last updated: 2026-11-27 (after wave 2).

## 1. FFmpeg rebuild (first: users are affected now)

The shipped FFmpeg libraries are older than `tools/ffmpeg/build.sh`. They cannot read the PNG files that the
generation tools create, and they lack the hardware encoders and OpenH264. Until the rebuild, generated images do not
decode.

Run `tools/ffmpeg/build.sh` on each build machine: macOS arm64, and Windows x64 from MSYS2 with the MSVC environment.
Then run `tools/ffmpeg/check-license.sh` and `tools/package.sh`.

| Change | Tasks |
|---|---|
| fix-render-and-edit-correctness | 3.1, 3.2, 3.3 |
| extend-export-formats-and-queue | 1.1, 1.2, then the encoder cases of 2.3 and 2.4 |

Decision needed first: OpenH264 patent coverage (see "Decisions" below).

## 2. CI on macOS and Linux (approval, then CI does the work)

These rows need the owner's approval to push, and to set the repository variable `OVE_CI_OSES` to
`["linux","macos"]`. That uses GitHub-hosted runners, free for this public repository; see `docs/ci-runners.md`. No
hands-on work is needed after that.

| Change | Tasks |
|---|---|
| fix-render-and-edit-correctness | 1.3 (effect goldens on all three systems) |
| add-audio-mixing-and-monitoring | 7.3 |
| add-transcript-editing | 2.3 (macOS run) |
| add-background-tasks-and-media-derivatives | 1.3 (macOS thread-priority readback) |

## 3. Hands-on on the Mac (built app, VideoToolbox, signing)

| Change | Tasks |
|---|---|
| add-model-runtime | 1.2 (macOS spike row, signed build: dylib loading), 5.1, 5.2, 5.3 |
| fix-render-and-edit-correctness | 6.1 (macOS half) |
| unify-agent-tool-layer | 8.1, 8.2, 8.3 (macOS half; 8.2 also needs real provider keys) |
| add-edit-planners-and-links | 6.2 (macOS half) |
| add-transcript-editing | 1.2 (Metal), 6.1 (macOS half) |
| add-audio-mixing-and-monitoring | 7.1, 7.2 (macOS half) |
| extend-export-formats-and-queue | 6.1, 6.2, 6.3 (macOS half; re-measure ProRes with `prores_videotoolbox`, design D6a) |
| add-background-tasks-and-media-derivatives | 7.1, 7.2, 7.3 (macOS half) |

## 4. GPU and reference hardware

| Change | Tasks |
|---|---|
| add-transcript-editing | 1.2 (Vulkan on Windows), 6.3 (cut planning on both reference machines) |
| add-edit-planners-and-links | 5.3 (two-up viewer timing) |
| add-background-tasks-and-media-derivatives | 7.1 (4K 10-bit playback during background load) |

## 5. Real accounts and data

| Change | Tasks |
|---|---|
| add-ai-generation | 1.4 (live provider APIs with real keys) |
| add-semantic-media-search | 3.1 (recall@5/@10 on a labelled set of the owner's footage) |
| add-transcript-editing | 3.1 (precision and recall on a labelled 10-minute interview: supply the recording) |

## Decisions only the owner can make

- OpenH264 patent coverage. Cisco's patent licence covers only the binaries Cisco itself distributes, not an
  OpenH264 we compile into our FFmpeg build. Options: ship as built; download Cisco's binary at run time; or leave
  OpenH264 out.
- Licences for DirectML and SVT-AV1. Both stay off until decided.
- A host and signing key for the remote model catalog. It stays off until decided.
- A self-hosted mirror for the ONNX Runtime library. The official PyPI wheels are used meanwhile.
- Licence sign-off for the semantic search model (add-semantic-media-search task 1.1): SigLIP 2 weights and the
  tokenizer files. Release of semantic search is blocked on it. The model is a 0.76 GB opt-in download.
- extend-ai-generation-actions: the remote model catalog's daily fetch, and creating and storing its ed25519 release
  signing key (task 1.4), before that change starts. Also real provider keys for its live checks (5.1, 5.3).
- Semantic search option (b): host our own smaller SigLIP 2 export (0.56 GB instead of 0.76 GB, passes the 0.99
  parity gate) as an immutable, versioned asset, with NOTICE text for the modified Apache-2.0 weights. Hosting means
  publishing files, so it needs your approval and a location. Until then the conversion's fp16 files are used.
- Creating a public community skills repository.
- Any place agent feedback reports could be sent. Today they are copied by hand only.
