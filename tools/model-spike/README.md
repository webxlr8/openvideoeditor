# Model runtime spike

Tools for the runtime spike of the `add-model-runtime` change (tasks 1.1 and 1.2). The results and the decision are
in `openspec/changes/add-model-runtime/design.md` under Results.

Nothing here is part of the product build. Model files, fixtures and Python environments stay outside the repository.

## 1. Export and parity (task 1.1)

`export_models.py` exports Silero VAD (16 kHz) and Beat This (`small0` checkpoint) to ONNX and compares each export
with the upstream reference implementation on three fixtures per model. It also writes the bench inputs and the
reference outputs as raw float32 files.

```sh
uv venv .venv
uv pip install --python .venv/Scripts/python.exe --index-url https://download.pytorch.org/whl/cpu torch torchaudio
uv pip install --python .venv/Scripts/python.exe onnx onnxscript onnxruntime numpy soundfile soxr silero-vad \
    einops rotary-embedding-torch "git+https://github.com/CPJKU/beat_this"
.venv/Scripts/python.exe -I tools/model-spike/export_models.py \
    --speech apps/desktop/e2e-app/fixtures/speech.wav --out <empty dir outside the repo>
```

The last line prints the worst difference and PASS or FAIL against 1e-3.

## 2. Runtime bench (task 1.2)

`bench/` is a separate Cargo project (not a workspace member) so the candidate runtimes never enter the product's
lock file. Build it three times and run one model per process, so the peak memory belongs to that model:

```sh
cd tools/model-spike/bench
cargo build --release                    # baseline without a runtime, for the size difference
cargo build --release --features ort     # ONNX Runtime, library loaded at run time
cargo build --release --features tract   # pure-Rust runtime
model-spike-bench <out dir> vad|beat [threads] [path to the onnxruntime library]
```

The onnxruntime library for the ort build must match API version 24 or later (for example the library inside the
`onnxruntime` Python wheel, or a release archive from the ONNX Runtime project).
