"""Export and parity for the visual search model (add-semantic-media-search tasks 1.2 and 1.3).

Loads the released SigLIP 2 base checkpoint with the reference framework (transformers), checks every checkpoint file
against its pinned SHA-256 before loading anything, and loads weights only (safetensors, no remote code, offline).
Then it:

1. makes deterministic fixture pictures and texts and writes them, the reference preprocessed tensors (sampled), the
   reference token ids and the reference embeddings to --golden (read by `crates/ove-vision/tests/reference.rs`);
2. exports our own ONNX encoders (image: full precision, half precision, 8-bit; text: full precision, 8-bit) to --out;
3. runs every export, and the shipped encoders named by --shipped, through ONNX Runtime on the reference inputs and
   prints the worst cosine similarity against the reference embeddings, with PASS or FAIL against 0.99.

Run with the dev venv, isolated, from the repository root:

    $OVE_PYTHON -I tools/vision-export/siglip2_export.py --checkpoint <checkpoint folder> --out <empty folder> \
        --golden crates/ove-vision/tests/reference --shipped <folder with the shipped .onnx files>

Nothing here is part of the product build; checkpoints and exports stay outside the repository.
"""

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path

# Offline and without remote code before the framework is imported.
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"

import numpy as np  # noqa: E402
import onnxruntime as ort  # noqa: E402
import torch  # noqa: E402
from PIL import Image  # noqa: E402

# google/siglip2-base-patch16-256 at commit 3f9f96cb90da5dbc758b01813f2f6f1aee24c1ab.
PINNED = {
    "model.safetensors": "6125cacc01fa93bdc98a0c5101cefcd69b2ed1f8ab4f38d86f4ad5984f5dc863",
    "tokenizer.json": "cb9140fae3ac5122c972d37adf83e1248471a38147ad76f8215c8872c6fd8322",
    "tokenizer.model": "61a7b147390c64585d6c3543dd6fc636906c9af3865a5548f27f31aee1d4c8e2",
}
# Small JSON files are recorded (hash printed) rather than pinned: they hold settings, not code or weights.
SETTINGS = ["config.json", "preprocessor_config.json", "special_tokens_map.json", "tokenizer_config.json"]

TEXTS = [
    "a photo of a cat",
    "boat in a harbour at dusk",
    "a solid red background",
    "a black and white checkerboard pattern",
    "a yellow circle on a dark background",
    "The Price Is Right!",
    "café crème, naïve résumé",
    "a very long sentence " + "word " * 80,
]

GATE = 0.99


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def check_checkpoint(folder: Path) -> dict:
    """Refuses to go on unless every pinned file matches; returns the hash of every file used."""
    seen = {}
    for name, want in PINNED.items():
        got = sha256(folder / name)
        if got != want:
            sys.exit(f"{name}: SHA-256 {got} is not the pinned {want}; refusing to load it")
        seen[name] = got
    for name in SETTINGS:
        seen[name] = sha256(folder / name)
    for name in sorted(os.listdir(folder)):
        if name.endswith((".bin", ".pt", ".pth", ".pkl", ".py")):
            sys.exit(f"{name}: unexpected pickle or code file in the checkpoint folder; refusing to go on")
    return seen


def pictures() -> list:
    """Deterministic test pictures (name, RGB uint8 array)."""
    w, h = 320, 180
    yy, xx = np.mgrid[0:h, 0:w]

    def solid(c):
        return np.broadcast_to(np.array(c, np.uint8), (h, w, 3)).copy()

    out = [
        ("red", solid([220, 20, 20])),
        ("blue", solid([20, 40, 220])),
        ("stripes", np.where(((xx // 20) % 2 == 0)[..., None], 255, 0).repeat(3, -1).astype(np.uint8)),
        (
            "checkerboard",
            np.where((((xx // 30) + (yy // 30)) % 2 == 0)[..., None], 250, 10).repeat(3, -1).astype(np.uint8),
        ),
        ("gradient", np.stack([xx * 255 // w] * 3, -1).astype(np.uint8)),
    ]
    disc = ((xx - 160) ** 2 + (yy - 90) ** 2) < 60**2
    out.append(("disc", np.where(disc[..., None], np.array([250, 210, 0]), np.array([10, 10, 60])).astype(np.uint8)))
    rng = np.random.default_rng(7)
    out.append(("noise", rng.integers(0, 256, (h, w, 3), dtype=np.uint8)))
    # A portrait picture, so the non-square resize is covered.
    pw, ph = 200, 300
    py, px = np.mgrid[0:ph, 0:pw]
    out.append(
        ("portrait", np.stack([px * 255 // pw, py * 255 // ph, (px + py) * 255 // (pw + ph)], -1).astype(np.uint8))
    )
    return out


def normalise(v: np.ndarray) -> np.ndarray:
    return v / np.linalg.norm(v, axis=-1, keepdims=True)


def pooled(out):
    """The pooled embedding, whatever the framework version returns."""
    if isinstance(out, torch.Tensor):
        return out
    if getattr(out, "pooler_output", None) is not None:
        return out.pooler_output
    return out[1]


class VisionTower(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, pixel_values):
        return pooled(self.model.vision_model(pixel_values=pixel_values))


class TextTower(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, input_ids):
        return pooled(self.model.text_model(input_ids=input_ids))


def export(module, example, name: str, input_name: str, out: Path) -> Path:
    path = out / f"{name}.onnx"
    torch.onnx.export(
        module,
        (example,),
        str(path),
        input_names=[input_name],
        output_names=["pooler_output"],
        dynamic_axes={input_name: {0: "batch"}, "pooler_output": {0: "batch"}},
        opset_version=17,
        dynamo=False,
    )
    return path



def run(path: Path, feed: dict) -> np.ndarray:
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 4
    s = ort.InferenceSession(str(path), opts, providers=["CPUExecutionProvider"])
    names = [o.name for o in s.get_outputs()]
    outs = s.run(None, feed)
    pick = names.index("pooler_output") if "pooler_output" in names else 0
    return normalise(np.asarray(outs[pick], np.float32))


def worst_cosine(a: np.ndarray, b: np.ndarray) -> float:
    return float(np.min(np.sum(normalise(a) * normalise(b), axis=-1)))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--checkpoint", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--golden", type=Path)
    ap.add_argument("--shipped", type=Path)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    hashes = check_checkpoint(args.checkpoint)
    from transformers import AutoTokenizer, SiglipImageProcessor, SiglipModel

    model = SiglipModel.from_pretrained(
        args.checkpoint, local_files_only=True, use_safetensors=True, trust_remote_code=False
    ).eval()
    tok = AutoTokenizer.from_pretrained(args.checkpoint, local_files_only=True, trust_remote_code=False)
    proc = SiglipImageProcessor.from_pretrained(args.checkpoint, local_files_only=True)

    pics = pictures()
    pixel_values = proc(images=[Image.fromarray(p) for _, p in pics], return_tensors="np")["pixel_values"]
    ids = tok([t.lower() for t in TEXTS], padding="max_length", max_length=64, truncation=True, return_tensors="np")[
        "input_ids"
    ].astype(np.int64)
    with torch.no_grad():
        ref_img = normalise(VisionTower(model)(torch.from_numpy(pixel_values)).numpy())
        ref_txt = normalise(TextTower(model)(torch.from_numpy(ids)).numpy())
    logit_scale = float(model.logit_scale.exp())
    logit_bias = float(model.logit_bias)
    print(f"reference: {len(pics)} pictures, {len(TEXTS)} texts; logit scale {logit_scale:.5f}, bias {logit_bias:.6f}")

    if args.golden:
        args.golden.mkdir(parents=True, exist_ok=True)
        stride = 97
        golden = {
            "checkpoint": "google/siglip2-base-patch16-256@3f9f96cb90da5dbc758b01813f2f6f1aee24c1ab",
            "files": hashes,
            "framework": {"transformers": __import__("transformers").__version__, "torch": torch.__version__},
            "logitScale": logit_scale,
            "logitBias": logit_bias,
            "stride": stride,
            "pictures": [],
            "texts": [],
        }
        for (name, rgb), pv, emb in zip(pics, pixel_values, ref_img):
            Image.fromarray(rgb).save(args.golden / f"{name}.png", optimize=True)
            flat = pv.reshape(-1)
            golden["pictures"].append(
                {
                    "name": name,
                    "pixelSamples": [round(float(x), 6) for x in flat[::stride]],
                    "pixelMean": [round(float(c), 6) for c in pv.reshape(3, -1).mean(1)],
                    "embedding": [round(float(x), 6) for x in emb],
                }
            )
        for text, row, emb in zip(TEXTS, ids, ref_txt):
            golden["texts"].append(
                {"text": text, "ids": [int(x) for x in row], "embedding": [round(float(x), 6) for x in emb]}
            )
        (args.golden / "reference.json").write_text(json.dumps(golden, separators=(",", ":")) + "\n", newline="\n")
        print(f"golden written to {args.golden}")

    # Our own exports.
    from onnxruntime.quantization import QuantType, quantize_dynamic
    from onnxruntime.transformers.float16 import convert_float_to_float16
    import onnx

    vision = export(VisionTower(model), torch.from_numpy(pixel_values[:1]), "vision_model", "pixel_values", args.out)
    text = export(TextTower(model), torch.from_numpy(ids[:1]), "text_model", "input_ids", args.out)
    m16 = convert_float_to_float16(onnx.load(str(vision)), keep_io_types=True)
    onnx.save(m16, str(args.out / "vision_model_fp16.onnx"))
    for src, kinds in [(vision, ["MatMul"]), (text, ["MatMul", "Gather"])]:
        quantize_dynamic(
            str(src),
            str(args.out / f"{src.stem}_int8.onnx"),
            per_channel=True,
            weight_type=QuantType.QInt8,
            op_types_to_quantize=kinds,
        )

    rows = []
    feeds = {"vision": {"pixel_values": pixel_values.astype(np.float32)}, "text": {"input_ids": ids}}
    refs = {"vision": ref_img, "text": ref_txt}
    candidates = [
        ("ours", "vision", args.out / "vision_model.onnx"),
        ("ours", "vision", args.out / "vision_model_fp16.onnx"),
        ("ours", "vision", args.out / "vision_model_int8.onnx"),
        ("ours", "text", args.out / "text_model.onnx"),
        ("ours", "text", args.out / "text_model_int8.onnx"),
    ]
    if args.shipped:
        candidates += [
            ("shipped", "vision", args.shipped / "vision_model_fp16.onnx"),
            ("shipped", "text", args.shipped / "text_model_q4f16.onnx"),
        ]
    for origin, kind, path in candidates:
        if not path.exists():
            continue
        # One picture or text per run: the community exports take a batch of one only for text.
        got = np.concatenate([run(path, {k: v[i : i + 1] for k, v in feeds[kind].items()}) for i in range(len(refs[kind]))])
        c = worst_cosine(got, refs[kind])
        size = path.stat().st_size
        rows.append((origin, path.name, size, c))
        print(f"{origin:8} {path.name:28} {size / 1e6:8.1f} MB  worst cosine {c:.4f}  {'PASS' if c >= GATE else 'FAIL'}")
    (args.out / "parity.json").write_text(json.dumps([dict(zip(["origin", "file", "bytes", "worstCosine"], r)) for r in rows], indent=1))


if __name__ == "__main__":
    main()
