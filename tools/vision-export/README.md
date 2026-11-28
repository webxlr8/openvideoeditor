# Visual search model export and parity

Tools for the visual search model of `add-semantic-media-search` (tasks 1.2 and 1.3). The results are in
`openspec/changes/add-semantic-media-search/design.md` under Results. Nothing here is part of the product build;
checkpoints, exports and Python environments stay outside the repository.

## Setup

Use the dev venv (`OVE_PYTHON`, set by the env script) with the packages of `tools/model-spike/requirements.txt` and
this folder's `requirements.txt`, installed by hash:

```sh
$OVE_PYTHON -I -m pip install --require-hashes -r tools/vision-export/requirements.txt
```

The checkpoint is the released `google/siglip2-base-patch16-256` at commit
`3f9f96cb90da5dbc758b01813f2f6f1aee24c1ab`, downloaded into a folder of its own (`config.json`,
`preprocessor_config.json`, `special_tokens_map.json`, `tokenizer_config.json`, `tokenizer.json`, `tokenizer.model`,
`model.safetensors`). The script checks the weights and tokenizer files against their pinned SHA-256 before loading
anything, refuses pickle or code files, loads safetensors weights only, offline and without remote code.

## Run

```sh
$OVE_PYTHON -I tools/vision-export/siglip2_export.py --checkpoint <checkpoint folder> --out <empty folder> \
    --golden crates/ove-vision/tests/reference --shipped <folder with the shipped .onnx files>
$OVE_PYTHON -I tools/vision-export/text_variants.py <the same --out folder>
$OVE_PYTHON -I tools/vision-export/per_text.py crates/ove-vision/tests/reference/reference.json <text .onnx>...
```

- `siglip2_export.py` writes the golden file and fixture pictures that `crates/ove-vision/tests/reference.rs` reads,
  exports our encoders (image: full, half precision, 8-bit; text: full, 8-bit) and prints each one's worst cosine
  against the reference framework with PASS or FAIL against 0.99.
- `text_variants.py` makes the smaller text encoders: the embedding table in 8 bits only, then the rest in half
  precision.
- `per_text.py` prints one text encoder's cosine per fixture text.

Then run our Rust pipeline (our preprocessing and tokenizer, ONNX Runtime) against the same golden file:

```sh
OVE_SIGLIP_DIR=<folder with tokenizer.model and the encoders> OVE_ONNXRUNTIME_LIB=<onnxruntime library> \
OVE_SIGLIP_VISION=<image encoder file> OVE_SIGLIP_TEXT=<text encoder file> \
    cargo test -p ove-vision --test reference -- --ignored --nocapture
```
