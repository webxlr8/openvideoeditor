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

(Filled in during apply: spike table per runtime and model — loads, parity, real-time factor, peak RSS, binary size —
on both reference machines.)
