"""Smaller text encoders made from our own full-precision export (siglip2_export.py --out), for the size/parity
trade-off: 8-bit embedding table only, then the rest in half precision. Prints each file's size.
Usage: $OVE_PYTHON -I tools/vision-export/text_variants.py <export folder>"""

import sys
from pathlib import Path

import onnx
from onnxruntime.quantization import QuantType, quantize_dynamic
from onnxruntime.transformers.float16 import convert_float_to_float16


def main() -> None:
    out = Path(sys.argv[1])
    full = out / "text_model.onnx"
    emb8 = out / "text_model_emb8.onnx"
    # Only the embedding lookup is quantised (per row); every matrix product keeps full precision.
    quantize_dynamic(str(full), str(emb8), weight_type=QuantType.QInt8, op_types_to_quantize=["Gather"], per_channel=True)
    # The same, then every remaining float weight in half precision (inputs and outputs stay as they are).
    # The dequantisation of the embedding table stays in float32 (its scales must stay float32).
    from onnxruntime.transformers.float16 import DEFAULT_OP_BLOCK_LIST

    block = list(DEFAULT_OP_BLOCK_LIST) + ["DequantizeLinear"]
    m = convert_float_to_float16(onnx.load(str(emb8)), keep_io_types=True, op_block_list=block)
    onnx.save(m, str(out / "text_model_emb8_fp16.onnx"))
    m = convert_float_to_float16(onnx.load(str(full)), keep_io_types=True)
    onnx.save(m, str(out / "text_model_fp16.onnx"))
    for p in sorted(out.glob("text_model*.onnx")):
        print(f"{p.name:32} {p.stat().st_size / 1e6:8.1f} MB")


if __name__ == "__main__":
    main()
