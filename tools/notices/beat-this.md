# Beat model: Beat This (small0)

Used by: beat and downbeat detection (`ove audio beats`, `detect_beats`, "Snap to beats"), change
add-audio-intelligence.

## Licence

The Beat This code and its published model weights are released under the MIT licence. The project README says:
"The code and the published model weights are released under the MIT license". Copyright (c) 2024 Institute of
Computational Perception, JKU Linz, Austria. Checked 2026-11-27 at https://github.com/CPJKU/beat_this.

MIT License

Copyright (c) 2024 Institute of Computational Perception, JKU Linz, Austria

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the Software without restriction, including without limitation the
rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the
Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Pins

The app does not use the checkpoint directly. `tools/model-spike/export_beat_this.py` exports it to two ONNX graphs
(our own frontend and the network); the app checks both against these pins before loading them.

| File | Bytes | SHA-256 |
| --- | --- | --- |
| `small0.ckpt` (source checkpoint, upstream) | 8,451,101 | `6074be2c4d490c5f6101fcc374a1ec72ae93456e23bb6019783b849f5dc7d47b` |
| `beat_this_frontend.onnx` | 4,467,329 | `3372aaf12dc26b763c33dc0982d2ded866bcc569e84b3ffd38e75b8b08a1fb01` |
| `beat_this_small0.onnx` | 10,555,597 | `71793320633424fb60ad2361a9ff2ee1e7e0c654b1991446855a1cf99208c727` |

Hosting: the exported files have no immutable download location yet. Until the owner publishes them at a pinned URL
(a versioned release or a commit-pinned model repository) and adds them to the model catalog, they are installed by
placing them in `<models>/beat-this-small/`; the app verifies size and SHA-256 before use and never downloads them.
