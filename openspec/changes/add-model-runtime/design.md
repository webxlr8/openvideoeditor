# Design: add-model-runtime

## Context

See proposal.md (Why). Whisper already has a downloader with SHA pins, partial files, a cancel flag and an environment
override (`crates/ove-transcribe/src/models.rs`); this change generalises it. Requirements: specs/model-runtime.
Quality targets from docs/03-architecture-plan.md: installer ≤ 60 MB, idle RAM ≤ 250 MB.

## Goals / Non-Goals

**Goals:**
- One way to install, verify and remove every local model on both platforms.
- An inference path chosen by measurement, not by preference.
- No growth of the installer and no growth of idle memory.

**Non-Goals:**
- Choosing feature models (each feature change pins its own manifest).
- GPU acceleration as a requirement (it is an optimisation measured per feature).

## Decisions

### D1. Manifest format
A model is a manifest: id, version, purpose, licence, files `[{name, url, sha256, bytes}]`, runtime kind and
in-process flag. URLs point at an immutable revision (a commit or a versioned release asset), never a moving branch.
Manifests are compiled into the binary; an environment variable overrides the base URL for tests and offline mirrors.
Alternatives: a remote manifest index (needs signing and hosting, and adds a network call at start); per-feature
ad-hoc constants (what we have now; drifts).

### D2. Downloader
HTTP GET with `Range` resume into `<name>.part`, streaming SHA-256, rename on match, delete on mismatch. A free-space
check (size + 10 %) runs first. One download per model at a time; a cancel token is checked every chunk. Progress is
reported as bytes and percent. Uses the HTTP client and TLS verifier already in the workspace.
Alternatives: a download library with its own TLS stack (second TLS stack, larger binary); no resume (a 450 MB file on
a weak connection would rarely finish).

### D3. Store location and layout
`<app data>/models/<id>/<version>/<files>` plus `installed.json` (id, version, hashes, installed-at). Models are shared
by all projects. Whisper files are adopted in place: on first start the store hashes existing whisper files and records
them, so nothing downloads again.
Alternative: models inside each project bundle (duplicates large files per project).

### D4. Inference runtime: decided by a spike
Candidates:
- A dynamically loaded ONNX runtime (MIT), with the native library downloaded on first use like a model (about
  15–20 MB per platform). Broad operator coverage; optional hardware execution providers.
- A pure-Rust ONNX runtime (MIT/Apache-2.0). No native library and no download, but narrower operator coverage and CPU
  only.
The spike runs Silero VAD and Beat This (small variant) on both platforms and measures: operator support (does it load
and run), output parity with the reference implementation, real-time factor, peak memory and added binary size. The
default is the dynamically loaded runtime if it passes and the pure-Rust runtime fails any model; otherwise the
pure-Rust runtime wins on size. The result and numbers go in Results. A feature may still use the other runtime when a
crate it depends on brings it in (for example the denoiser), but only through the adapter in D5.
Alternatives considered and rejected without a spike: a statically linked ONNX runtime in the installer (15–20 MB per
platform breaks the size budget); platform-only runtimes (Core ML, DirectML-only) (no shared code path, no CPU parity).

### D5. Adapter and worker protocol
`ove-models` exposes one trait: load a model by manifest id, run named tensors in and out. Models flagged in-process
(≤ 10 MB) run in the editor process. Others run in `ove model-worker`, a child process speaking length-prefixed JSON
headers plus raw tensor bytes over stdio. The worker loads models lazily, serves one job at a time per model, and exits
after 120 s idle. The editor restarts it on demand and fails the job on a crash.
Alternatives: threads in the editor process (memory stays high after use; a model crash kills the editor); a local
HTTP server (ports, firewall prompts, more code).

### D6. Consent gate
The store knows each manifest's total bytes. `ensure_installed(id, origin)` returns `NeedsConsent{bytes}` when the
total is over 50 MB and the origin is not the person in the app. The app shows a dialog; the CLI prints the install
command (`ove models install <id>`), which needs `--yes` for large models. MCP returns the same result as data.
Alternative: always auto-download (surprises people on metered connections and fills disks).

### D7. Surfaces
Settings → Storage section "Models" (name, purpose, size, location, Remove). CLI `ove models list|install|remove
[--json]`. MCP `list_models_local` (read-only). Exit codes follow docs/spec/cli.md (0 ok, 1 refused).

## Risks / Trade-offs

- Mirror or host removes a file → installs fail. Mitigation: commit-pinned URLs plus the base-URL override; a weekly
  CI job fetches each manifest URL with a HEAD request. Detection: that job fails.
- Native runtime library must be signed and notarised on macOS and may be flagged on Windows. Mitigation: download it
  through the same signed-asset path as other release files; until signing exists (`add-distribution`), document the
  first-run prompt. Detection: smoke test on clean macOS and Windows machines.
- Worker protocol overhead on large tensors. Mitigation: raw bytes after a small JSON header, no base64. Detection:
  benchmark of a 256×256×3 image tensor round trip ≤ 2 ms.
- Antivirus on Windows quarantines the worker. Mitigation: the worker is the same signed `ove` binary, not a new
  executable. Detection: Windows smoke test with Defender on.

## Migration Plan

Whisper files are adopted in place (D3). No project data changes. Removing this change means features fall back to
"model not installed".

## Open Questions

- Waves proceed in the order of the master roadmap; this change is in wave 1 and blocks the model-based features.
- Models and the runtime library download on first use, with explicit consent above 50 MB (D6). The owner should
  confirm this pattern for the installer budget.
- Hardware execution providers on Windows (DirectML) stay off by default, with the CPU path as fallback, until the
  owner rules on the DirectML redistribution licence.
- Each feature change must check that its model weights (not only the code) carry a licence allowed by
  openspec/config.yaml before pinning them.

## Results

### R1. Weight licences (task 1.1)

| Model | Code licence | Weight licence | Evidence |
|---|---|---|---|
| Silero VAD 16 kHz (package 6.3) | MIT | MIT | The weights ship inside the package with its `LICENSE` file ("MIT License, Copyright (c) 2020-present Silero Team"); the package metadata classifies it as MIT. The project README shows a badge whose alt text says "CC BY-NC 4.0" while the badge itself and the licence file say MIT; the licence file governs, but a feature that pins this model should re-check it. |
| Beat This, `small0` checkpoint (package 1.1.0) | MIT | MIT | Project README, License section: "The code and the published model weights are released under the MIT license." The README notes that some training data has other licences; that does not bind the weights. Checkpoints are served from the authors' university file share (`cloud.cp.jku.at/public.php/dav/files/7ik4RrBKTS273gp/<name>.ckpt`), which is not commit-pinned: a feature that pins this model must mirror the file on a versioned release asset. |
| Whisper ggml (tiny, base, small) | MIT | MIT | Model card of the whisper.cpp model repository declares `license: mit`. The files are now fetched from commit `5359861c739e955e79d9a303bcbc70fb988958b1` instead of the moving `main` branch; the size and SHA-256 of `ggml-tiny.bin` at that commit match the pins already in the code. |

### R2. Export and parity (task 1.1)

Our own exporter (`tools/model-spike/export_models.py`, torch 2.15.0, opset 17, onnxruntime 1.32.0 in Python):

- Silero VAD: the network is rebuilt as our own PyTorch module, loaded with the weights from the upstream TorchScript
  file and exported. One 32 ms step: 64 samples of context plus 512 new samples and the recurrent state in;
  speech probability and new state out. The LSTM cell is written out and `squeeze` is replaced by indexing, so the
  graph holds only plain operators (see R3). File: 1,247,273 bytes.
- Beat This `small0`: upstream's module exported with a fixed input of 1500 spectrogram frames (30 s, the chunk size
  upstream inference uses). File: 9,961,009 bytes. The log-mel spectrogram stays outside the model.

Reference: the upstream implementation in PyTorch (TorchScript model for Silero, run the way upstream runs it; the
PyTorch module for Beat This). Fixtures: 30 s each, made by the script: repeated speech, speech with white noise,
synthetic music (Silero); two synthetic drum loops at 100 and 128 BPM and the speech (Beat This).

| Model | Fixture | Max absolute difference | Output compared |
|---|---|---|---|
| Silero VAD | speech | 1.64e-4 | speech probability (the model's output is after the sigmoid) |
| Silero VAD | speech + noise | 2.2e-6 | speech probability |
| Silero VAD | music | 5.1e-7 | speech probability |
| Beat This small0 | drums 100 BPM | 2.9e-5 | beat and downbeat logits |
| Beat This small0 | drums 128 BPM | 6.2e-5 | beat and downbeat logits |
| Beat This small0 | speech | 1.4e-5 | beat and downbeat logits |

All six are below the 1e-3 limit.

### R3. Runtime spike on Windows (task 1.2)

Machine: Windows 11 x64, AMD Ryzen 9 5950X (16 cores), 128 GB. Bench: `tools/model-spike/bench`, release profile
as the workspace (`lto = "thin"`, `codegen-units = 1`), one model per process, median of 3 runs, 90 s of audio per
model (three fixtures). Candidates: `ort` 2.0.0-rc.15 with `load-dynamic` and the onnxruntime 1.32.0 library;
`tract-onnx` 0.23.10. Parity is against the PyTorch reference outputs from R2.

| Runtime | Model | Loads | Max abs. difference | Load time | Real-time factor | Faster than real time | Peak RSS |
|---|---|---|---|---|---|---|---|
| onnxruntime, 1 thread | Silero VAD | yes | 1.64e-4 | 14 ms | 0.0027 | 368× | 37 MB |
| onnxruntime, 4 threads | Silero VAD | yes | 1.64e-4 | 13 ms | 0.0026 | 381× | 37 MB |
| tract, 1 thread | Silero VAD | yes (after the export change below) | 1.62e-4 | 16 ms | 0.0021 | 476× | 15 MB |
| onnxruntime, 1 thread | Beat This small0 | yes | 6.2e-5 | 227 ms | 0.036 | 28× | 566 MB |
| onnxruntime, 4 threads | Beat This small0 | yes | 6.2e-5 | 302 ms | 0.016 | 63× | 567 MB |
| tract, 1 thread | Beat This small0 | yes | 8.4e-5 | 368 ms | 0.075 | 13× | 600 MB |

Added size (Windows x64 release binary, against the same bench without a runtime, 163,328 bytes):

| Runtime | Added to the binary | Compressed (zip, deflate 9) | Downloaded on first use |
|---|---|---|---|
| onnxruntime (load-dynamic) | +208 KB | +83 KB | `onnxruntime.dll` 22,668,640 bytes (7.6 MB compressed) |
| tract | +25.3 MB | +8.1 MB | nothing |

Findings:
- Both runtimes load and run both models within tolerance on the CPU. tract first rejected the Silero export: the
  exporter had turned `squeeze` into an `If` node whose two branches have different ranks. Replacing `squeeze` with
  indexing fixed it. This is a sign of tract's narrower operator coverage: feature models must be exported with
  tract in mind if tract is used.
- Speed: for the tiny streaming model both are far faster than needed (tract slightly faster, it has less per-call
  overhead). For the transformer, onnxruntime is 2.1× faster on one thread and 4.8× faster with 4 threads (tract runs
  single-threaded here).
- Memory: Beat This on a full 30 s chunk peaks near 570 to 600 MB in both runtimes (attention over 1500 frames).
  This confirms D5: such a model must run in the worker, not in the editor. A feature can lower the peak with shorter
  chunks.
- Size: the measured size result is the opposite of the assumption in D4. tract is compiled into the binary and adds
  8.1 MB compressed to every installer (the CLI is in the installer, and the worker is the same binary), which breaks
  task 5.3 (installer unchanged within 1 MB). onnxruntime adds 83 KB compressed to the installer; its library is
  downloaded on first use like a model (7.6 MB compressed, 22.7 MB on disk on Windows).
- Not measured: macOS (Apple silicon). The table must be repeated there (`tools/model-spike/README.md`).

### R4. Decision (task 1.3, awaiting owner sign-off)

The D4 rule assumed that the pure-Rust runtime is the smaller one. Measured, it is not: it adds about 25 MB to the
binary, while the dynamically loaded runtime adds 0.2 MB and downloads its library on first use. Both pass every model.
Recommendation: the dynamically loaded ONNX runtime is the default, because it is the only candidate that keeps the
installer within 1 MB (task 5.3) and it is 2 to 5 times faster on the transformer model. tract stays available only
where a feature's own dependency brings it in, through the adapter (D5). The runtime library is downloaded through
the model store with the same pins and consent rules as a model (it is under 50 MB, so it may download on first use
with visible progress). This needs the owner's sign-off and the macOS row before task 1.3 is ticked.

### R5. Adapter and worker (tasks 3.1 and 3.2, Windows)

- Protocol: a 4-byte length, a JSON header, then the raw little-endian tensor bytes; on both shipped platforms the
  in-memory layout is the wire layout, so tensors are written and read without per-element conversion.
- Round trip of a 1×256×256×3 float tensor through `ove model-worker` (echo model, 50 runs after 5 warm-up runs):
  median 0.56 ms (min 0.52, max 0.96) in the test profile; median 0.69 ms in a release build. Limit: 2 ms. A first
  version that converted each element took 3.5 ms in release, which is why the byte view exists.
- Idle exit: with `--idle-secs 1` the worker had exited within 5 s of its last job, and the next job started a new
  process. The default is 120 s.
- Crash: killing the worker during a 10 s job failed that job within 3 s with "the model worker stopped during the
  job"; the next job started a new worker and succeeded. A job that fails inside a healthy worker (for example a
  model that cannot load) keeps the worker.
- Parity through the adapter (ONNX Runtime 1.32.0 library): Silero VAD 1.64e-4 and Beat This 6.2e-5 against the
  PyTorch reference, the same in process and in the worker. 2,814 VAD steps plus 3 Beat This chunks took 2.7 s in
  the worker.

### R6. Runtime library download (task 3.3, Windows)

Decided at gate 1: ONNX Runtime 1.32.0 from the official wheels on the Python package index. The file locations
there are content addressed (`/packages/<2>/<2>/<60 hex>/<file>`), so the manifest check accepts them as immutable.

| Platform | Wheel (download) | Library (installed) | Notices installed next to it |
|---|---|---|---|
| Windows x64 | 16,698,767 bytes | `onnxruntime.dll`, 22,668,640 bytes | `LICENSE` 1,094 bytes, `ThirdPartyNotices.txt` 341,120 bytes |
| macOS arm64 (14 or later) | 22,519,308 bytes | `libonnxruntime.1.32.0.dylib`, 35,272,344 bytes | `LICENSE` 1,073 bytes, `ThirdPartyNotices.txt` 334,795 bytes |

- Every member is pinned by size and SHA-256 as well as the wheel; both real wheels pass an offline install with
  those pins (`the_pinned_wheel_installs_offline`).
- Consent: the runtime is native code from a third-party host, so it downloads only after the person's action
  (Settings → Storage Install, or `--yes` at the terminal), although it is under 50 MB. A feature or agent gets
  "runtime not installed" with the download size, the host and the install command.
- Clean-profile install on Windows (debug `ove`, home connection): "runtime not installed" (exit 1) without
  `--yes`; with `--yes` the 16.7 MB wheel downloaded, verified and extracted in 5.3 s; the store then holds the
  library, `LICENSE` and `ThirdPartyNotices.txt` (23.0 MB) and no wheel. `ove model-worker` with no
  `OVE_ONNXRUNTIME_LIB` loaded that library from the store.
- Added dependency: `zip` 9.8.2 (MIT), `deflate` feature only (pulls `flate2` with `zlib-rs`, and `zopfli`).
- `tract` stays buildable as the `ove-models` feature `tract`; the nightly job builds it. It is not in any shipped
  binary.
- Not measured here: the macOS download and load (needs the macOS machine; part of 5.1).
