"""Per-input cosine of one text encoder against the reference embeddings in a golden file (a diagnosis aid for
siglip2_export.py). Usage: $OVE_PYTHON -I tools/vision-export/per_text.py <reference.json> <text encoder .onnx>..."""

import json
import sys

import numpy as np
import onnxruntime as ort


def main() -> None:
    golden = json.load(open(sys.argv[1], encoding="utf-8"))
    for path in sys.argv[2:]:
        s = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
        names = [o.name for o in s.get_outputs()]
        pick = names.index("pooler_output") if "pooler_output" in names else 0
        print(path)
        for t in golden["texts"]:
            ids = np.array([t["ids"]], np.int64)
            v = np.asarray(s.run(None, {"input_ids": ids})[pick], np.float32)[0]
            v /= np.linalg.norm(v)
            ref = np.array(t["embedding"], np.float32)
            print(f"  {float(v @ ref):.4f}  {t['text'][:40]!r}")


if __name__ == "__main__":
    main()
