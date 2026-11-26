"""Runtime spike for add-model-runtime, task 1.1.

Exports the two reference models to ONNX with our own code and checks each export against the upstream reference
implementation on three fixtures:

- Silero VAD (16 kHz): the upstream package ships TorchScript and ONNX files. We do not reuse its ONNX file. We
  rebuild the network as a small PyTorch module, load the weights from the upstream TorchScript file, export that
  module, and compare it with the upstream TorchScript model run the way upstream runs it (512-sample chunks,
  64 samples of context, recurrent state carried between chunks).
- Beat This (small0 checkpoint): the network is upstream's PyTorch module. We export it with a fixed input of
  1500 spectrogram frames (30 s at 50 frames per second, the chunk size upstream inference uses) and compare
  the beat and downbeat logits.

The script also writes the inputs and the reference outputs as raw little-endian float32 files, so the Rust bench
(`bench/`) can check parity for each runtime without Python.

Usage (from any directory; nothing is written inside the repository):
    python -I export_models.py --speech <16 kHz mono wav> --out <empty output dir>

Needs: torch, torchaudio, onnx, onnxscript, onnxruntime, numpy, soundfile, silero-vad, beat_this.
"""

import argparse
import json
import math
import os
import sys
import time

import numpy as np
import onnxruntime as ort
import soundfile as sf
import torch
from torch import nn

SR_VAD = 16000
CHUNK = 512
CONTEXT = 64
SR_BEAT = 22050
BEAT_FRAMES = 1500


# ---------------------------------------------------------------------------------------------------------------------
# Fixtures. All three are made here so the spike does not depend on downloaded audio.


def synth_music(seconds: float, bpm: float, sr: int, seed: int) -> np.ndarray:
    """A plain drum-and-chord loop: kick on every beat, accent on beat one, hi-hat on off-beats, a held chord."""
    rng = np.random.default_rng(seed)
    n = int(seconds * sr)
    out = np.zeros(n, dtype=np.float64)
    beat = 60.0 / bpm
    t_kick = np.arange(int(0.15 * sr)) / sr
    kick = np.sin(2 * math.pi * (60 + 90 * np.exp(-t_kick * 30)) * t_kick) * np.exp(-t_kick * 18)
    hat = rng.standard_normal(int(0.04 * sr)) * np.exp(-np.arange(int(0.04 * sr)) / sr * 90) * 0.25
    i = 0
    while True:
        start = int(i * beat * sr)
        if start >= n:
            break
        gain = 1.0 if i % 4 == 0 else 0.6
        end = min(n, start + kick.size)
        out[start:end] += gain * kick[: end - start]
        off = int((i + 0.5) * beat * sr)
        if off < n:
            end = min(n, off + hat.size)
            out[off:end] += hat[: end - off]
        i += 1
    t = np.arange(n) / sr
    for f in (220.0, 277.18, 329.63):
        out += 0.05 * np.sin(2 * math.pi * f * t)
    return (out / np.max(np.abs(out)) * 0.8).astype(np.float32)


def resample(x: np.ndarray, sr_in: int, sr_out: int) -> np.ndarray:
    import soxr

    return soxr.resample(x, sr_in, sr_out).astype(np.float32)


def make_fixtures(speech_path: str) -> dict:
    speech, sr = sf.read(speech_path, dtype="float32")
    if speech.ndim == 2:
        speech = speech.mean(axis=1)
    if sr != SR_VAD:
        speech = resample(speech, sr, SR_VAD)
    rng = np.random.default_rng(7)
    # 30 s of speech: the clip repeated with short gaps of silence.
    gap = np.zeros(SR_VAD // 2, dtype=np.float32)
    long_speech = np.concatenate([np.concatenate([speech, gap]) for _ in range(math.ceil(30 * SR_VAD / (speech.size + gap.size)))])[: 30 * SR_VAD]
    noisy = long_speech + rng.standard_normal(long_speech.size).astype(np.float32) * 0.02
    vad = {
        "speech": long_speech,
        "speech_noise": np.clip(noisy, -1, 1).astype(np.float32),
        "music": resample(synth_music(30, 120, SR_BEAT, 1), SR_BEAT, SR_VAD),
    }
    beat = {
        "drums100": synth_music(30, 100, SR_BEAT, 2),
        "drums128": synth_music(30, 128, SR_BEAT, 3),
        "speech": resample(long_speech, SR_VAD, SR_BEAT),
    }
    return {"vad": vad, "beat": beat}


# ---------------------------------------------------------------------------------------------------------------------
# Silero VAD: our own module with the upstream weights.


class SileroVad16k(nn.Module):
    """One 16 kHz step: 64 samples of context plus 512 new samples in, speech probability and new state out."""

    def __init__(self):
        super().__init__()
        self.register_buffer("basis", torch.zeros(258, 1, 256))
        self.convs = nn.ModuleList(
            [
                nn.Conv1d(129, 128, 3, padding=1),
                nn.Conv1d(128, 64, 3, stride=2, padding=1),
                nn.Conv1d(64, 64, 3, stride=2, padding=1),
                nn.Conv1d(64, 128, 3, padding=1),
            ]
        )
        self.cell = nn.LSTMCell(128, 128)
        self.out = nn.Conv1d(128, 1, 1)

    def load_upstream(self, jit_state: dict):
        p = "_model."
        sd = {"basis": jit_state[p + "stft.forward_basis_buffer"]}
        for i in range(4):
            sd[f"convs.{i}.weight"] = jit_state[f"{p}encoder.{i}.reparam_conv.weight"]
            sd[f"convs.{i}.bias"] = jit_state[f"{p}encoder.{i}.reparam_conv.bias"]
        for k in ("weight_ih", "weight_hh", "bias_ih", "bias_hh"):
            sd[f"cell.{k}"] = jit_state[f"{p}decoder.rnn.{k}"]
        sd["out.weight"] = jit_state[p + "decoder.decoder.2.weight"]
        sd["out.bias"] = jit_state[p + "decoder.decoder.2.bias"]
        self.load_state_dict(sd)

    def forward(self, x: torch.Tensor, state: torch.Tensor):
        x = nn.functional.pad(x, (0, 64), mode="reflect").unsqueeze(1)
        x = nn.functional.conv1d(x, self.basis, stride=128)
        x = torch.sqrt(x[:, :129, :] ** 2 + x[:, 129:, :] ** 2)
        for conv in self.convs:
            x = torch.relu(conv(x))
        # Indexing instead of squeeze(-1): the exporter turns squeeze on a traced dimension into an If node, which
        # the pure-Rust runtime rejects (its two branches have different ranks). Indexing exports as Gather.
        x = x[:, :, 0]
        # The cell written out (instead of calling the module) so the exporter emits plain MatMul and Slice nodes.
        gates = x @ self.cell.weight_ih.T + self.cell.bias_ih + state[0] @ self.cell.weight_hh.T + self.cell.bias_hh
        i, f = torch.sigmoid(gates[:, 0:128]), torch.sigmoid(gates[:, 128:256])
        g, o = torch.tanh(gates[:, 256:384]), torch.sigmoid(gates[:, 384:512])
        c = f * state[1] + i * g
        h = o * torch.tanh(c)
        y = torch.sigmoid(self.out(torch.relu(h).unsqueeze(-1)))
        return y[:, 0, :].mean(dim=1, keepdim=True), torch.stack([h, c])


def silero_reference(jit_model, audio: np.ndarray) -> np.ndarray:
    jit_model.reset_states()
    pad = (-audio.size) % CHUNK
    x = torch.from_numpy(np.pad(audio, (0, pad)))
    probs = []
    with torch.no_grad():
        for i in range(0, x.numel(), CHUNK):
            probs.append(float(jit_model(x[i : i + CHUNK], SR_VAD)))
    return np.array(probs, dtype=np.float32)


def silero_chunks(audio: np.ndarray) -> np.ndarray:
    """The exact step inputs the exported model sees: [n, 576] rows of context + chunk."""
    pad = (-audio.size) % CHUNK
    x = np.concatenate([np.zeros(CONTEXT, np.float32), np.pad(audio, (0, pad))])
    n = (x.size - CONTEXT) // CHUNK
    return np.stack([x[i * CHUNK : i * CHUNK + CHUNK + CONTEXT] for i in range(n)]).astype(np.float32)


def silero_onnx_run(path: str, rows: np.ndarray) -> np.ndarray:
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 1
    sess = ort.InferenceSession(path, opts, providers=["CPUExecutionProvider"])
    state = np.zeros((2, 1, 128), np.float32)
    probs = []
    for row in rows:
        p, state = sess.run(None, {"input": row[None, :], "state": state})
        probs.append(float(p[0, 0]))
    return np.array(probs, dtype=np.float32)


# ---------------------------------------------------------------------------------------------------------------------
# Beat This.


class BeatHeads(nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, spect):
        y = self.model(spect)
        return y["beat"], y["downbeat"]


def beat_spect(audio: np.ndarray) -> np.ndarray:
    from beat_this.preprocessing import LogMelSpect

    with torch.no_grad():
        s = LogMelSpect()(torch.from_numpy(audio)).numpy().astype(np.float32)
    if s.shape[0] < BEAT_FRAMES:
        s = np.pad(s, ((0, BEAT_FRAMES - s.shape[0]), (0, 0)))
    return s[:BEAT_FRAMES]


# ---------------------------------------------------------------------------------------------------------------------


def write_f32(path: str, a: np.ndarray):
    np.ascontiguousarray(a, dtype="<f4").tofile(path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--speech", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    torch.manual_seed(0)
    fixtures = make_fixtures(args.speech)
    report = {"torch": torch.__version__, "onnxruntime": ort.__version__, "models": {}}

    # --- Silero VAD
    from silero_vad import load_silero_vad

    jit = load_silero_vad()
    vad = SileroVad16k().eval()
    vad.load_upstream(jit.state_dict())
    vad_path = os.path.join(args.out, "silero_vad_16k.onnx")
    torch.onnx.export(
        vad,
        (torch.zeros(1, CHUNK + CONTEXT), torch.zeros(2, 1, 128)),
        vad_path,
        input_names=["input", "state"],
        output_names=["prob", "state_out"],
        opset_version=17,
        dynamo=False,
    )
    entries = []
    for name, audio in fixtures["vad"].items():
        ref = silero_reference(jit, audio)
        rows = silero_chunks(audio)
        got = silero_onnx_run(vad_path, rows)
        diff = float(np.max(np.abs(ref - got)))
        write_f32(os.path.join(args.out, f"vad_{name}.in.f32"), rows)
        write_f32(os.path.join(args.out, f"vad_{name}.ref.f32"), ref)
        entries.append({"fixture": name, "steps": int(rows.shape[0]), "seconds": audio.size / SR_VAD, "max_abs_diff": diff, "speech_frames": int((ref > 0.5).sum())})
    report["models"]["silero_vad_16k"] = {"file": vad_path, "bytes": os.path.getsize(vad_path), "fixtures": entries}

    # --- Beat This small0
    from beat_this.inference import load_model

    bt = BeatHeads(load_model("small0", "cpu")).eval()
    beat_path = os.path.join(args.out, "beat_this_small0.onnx")
    torch.onnx.export(
        bt,
        (torch.zeros(1, BEAT_FRAMES, 128),),
        beat_path,
        input_names=["spect"],
        output_names=["beat", "downbeat"],
        opset_version=17,
        dynamo=False,
    )
    sess = ort.InferenceSession(beat_path, providers=["CPUExecutionProvider"])
    entries = []
    for name, audio in fixtures["beat"].items():
        spect = beat_spect(audio)
        with torch.no_grad():
            rb, rd = bt(torch.from_numpy(spect)[None])
        rb, rd = rb.numpy()[0], rd.numpy()[0]
        t0 = time.perf_counter()
        gb, gd = sess.run(None, {"spect": spect[None]})
        ort_s = time.perf_counter() - t0
        diff = float(max(np.max(np.abs(rb - gb[0])), np.max(np.abs(rd - gd[0]))))
        write_f32(os.path.join(args.out, f"beat_{name}.in.f32"), spect)
        write_f32(os.path.join(args.out, f"beat_{name}.ref.f32"), np.concatenate([rb, rd]))
        entries.append({"fixture": name, "frames": BEAT_FRAMES, "seconds": BEAT_FRAMES / 50, "max_abs_diff": diff, "python_ort_seconds": ort_s, "beat_frames_over_0": int((rb > 0).sum())})
    report["models"]["beat_this_small0"] = {"file": beat_path, "bytes": os.path.getsize(beat_path), "fixtures": entries}

    with open(os.path.join(args.out, "export_report.json"), "w") as f:
        json.dump(report, f, indent=2)
    json.dump(report, sys.stdout, indent=2)
    worst = max(e["max_abs_diff"] for m in report["models"].values() for e in m["fixtures"])
    print(f"\nworst max abs diff: {worst:.3g} ({'PASS' if worst < 1e-3 else 'FAIL'} against 1e-3)")


if __name__ == "__main__":
    main()
