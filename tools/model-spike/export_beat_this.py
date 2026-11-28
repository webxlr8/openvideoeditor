"""Beat model export and parity gate for add-audio-intelligence, task 3.1 (design D6).

Exports the Beat This "small0" checkpoint to two ONNX graphs, so the app feeds 22,050 Hz mono samples and gets beat
and downbeat logits at 50 frames per second:

- beat_this_frontend.onnx: our own log-mel frontend. The STFT is a strided convolution with a fixed Hann-windowed DFT
  basis, followed by the mel filter bank and log1p(1000 x). It takes samples that the caller has reflect-padded by
  512 at the ends of the piece, so it can run on any stretch of the piece and give the same frames.
- beat_this_small0.onnx: the network, from a spectrogram chunk to logits.

The model attends over a whole chunk, so its logits depend on where chunks start. The app splits the spectrogram the
way the network was trained and evaluated: 1,500-frame chunks starting 6 frames before the piece, 1,488 frames
apart, the last one moved to end at the piece's end, 6 border frames dropped from each, and earlier chunks win where
chunks overlap (crates/ove-analyze/src/beats.rs). The parity gate runs the upstream reference implementation and
this pipeline (both ONNX graphs) on synthetic fixtures, and reports the largest logit difference and whether the
picked beats and downbeats agree within one frame.

The checkpoint is treated as untrusted input: it is downloaded into its own folder, its SHA-256 is printed before it
is loaded, and it is loaded with torch.load(weights_only=True).

Usage (Python from the dev venv, OVE_PYTHON; nothing is written inside the repository):
    $OVE_PYTHON -I tools/model-spike/export_beat_this.py --out <empty dir outside the repo>
"""

import argparse
import hashlib
import inspect
import json
import os
import sys
import urllib.request

import numpy as np
import torch
import torch.nn.functional as F

SR = 22050
N_FFT = 1024
HOP = 441
CHUNK = 30 * SR
BORDER = 6
CKPT_URL = "https://cloud.cp.jku.at/public.php/dav/files/7ik4RrBKTS273gp/small0.ckpt"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


class Frontend(torch.nn.Module):
    """Log-mel spectrogram: centred frames (reflect padding), Hann window, magnitude normalised by the frame length,
    128 Slaney mel bands from 30 Hz to 11 kHz, then log1p(1000 x)."""

    def __init__(self, fbank):
        super().__init__()
        n = torch.arange(N_FFT, dtype=torch.float64)
        window = 0.5 - 0.5 * torch.cos(2 * torch.pi * n / N_FFT)  # periodic Hann
        k = torch.arange(N_FFT // 2 + 1, dtype=torch.float64)[:, None]
        angle = 2 * torch.pi * k * n[None, :] / N_FFT
        scale = 1.0 / np.sqrt(N_FFT)
        basis = torch.cat([torch.cos(angle) * window * scale, -torch.sin(angle) * window * scale], 0)
        self.register_buffer("basis", basis.float()[:, None, :])
        self.register_buffer("fbank", fbank.float())

    def forward(self, x):  # x: (1, samples), already reflect-padded at the piece ends
        spec = F.conv1d(x[:, None, :], self.basis, stride=HOP)  # (1, 2 * bins, frames)
        bins = N_FFT // 2 + 1
        mag = torch.sqrt(spec[:, :bins] ** 2 + spec[:, bins:] ** 2)
        mel = torch.matmul(mag.transpose(1, 2), self.fbank)  # (1, frames, 128)
        return torch.log1p(1000.0 * mel)


class Network(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, spect):
        out = self.model(spect)
        return out["beat"], out["downbeat"]


def click_track(bpm, beats_per_bar, seconds, seed):
    rng = np.random.default_rng(seed)
    t = np.arange(int(seconds * SR)) / SR
    x = 0.01 * rng.standard_normal(len(t))
    period = 60.0 / bpm
    k = 0
    while k * period < seconds - 0.1:
        a = int(k * period * SR)
        n = int(0.05 * SR)
        f = 1500.0 if k % beats_per_bar == 0 else 900.0
        env = np.exp(-np.arange(n) / (0.012 * SR))
        x[a : a + n] += (0.8 if k % beats_per_bar == 0 else 0.45) * env * np.sin(2 * np.pi * f * np.arange(n) / SR)
        # A low drum on the downbeat, a hat on the off-beat.
        if k % beats_per_bar == 0:
            x[a : a + n] += 0.6 * env * np.sin(2 * np.pi * 60 * np.arange(n) / SR)
        h = int((k + 0.5) * period * SR)
        x[h : h + n // 4] += 0.1 * rng.standard_normal(len(x[h : h + n // 4]))
        k += 1
    return x.astype(np.float32)


CHUNK_FRAMES = 1500
SPECT_SEGMENT = 3000  # frames per frontend run


def ours(front, net, samples):
    """The app's pipeline (crates/ove-analyze/src/beats.rs): frontend over the reflect-padded piece in segments, then
    the network over 1,500-frame spectrogram chunks."""
    half = N_FFT // 2
    padded = np.pad(samples, (half, half), mode="reflect").astype(np.float32)
    frames = 1 + len(samples) // HOP
    parts = []
    f0 = 0
    while f0 < frames:
        f1 = min(frames, f0 + SPECT_SEGMENT)
        seg = padded[f0 * HOP : (f1 - 1) * HOP + N_FFT]
        parts.append(front.run(None, {"samples": seg[None]})[0][0])
        f0 = f1
    spect = np.concatenate(parts)
    n = len(spect)
    starts = list(range(-BORDER, n - BORDER, CHUNK_FRAMES - 2 * BORDER))
    if n > CHUNK_FRAMES - 2 * BORDER:
        starts[-1] = n - (CHUNK_FRAMES - BORDER)
    beat = np.full(n, -1000.0, np.float32)
    down = np.full(n, -1000.0, np.float32)
    for start in reversed(starts):
        a, b = max(start, 0), min(start + CHUNK_FRAMES, n)
        left = max(0, -start)
        right = max(0, min(BORDER, start + CHUNK_FRAMES - n))
        chunk = np.pad(spect[a:b], ((left, right), (0, 0)))
        lb, ld = net.run(None, {"spect": chunk[None]})
        lb, ld = lb[0][BORDER:-BORDER], ld[0][BORDER:-BORDER]
        lo = start + BORDER
        beat[lo : lo + len(lb)] = lb[: max(0, n - lo)]
        down[lo : lo + len(ld)] = ld[: max(0, n - lo)]
    return beat, down


def peaks(logits, radius=3):
    out = []
    for i in range(len(logits)):
        if logits[i] < 0:  # sigmoid < 0.5
            continue
        lo, hi = max(0, i - radius), min(len(logits), i + radius + 1)
        if all(logits[j] < logits[i] or (logits[j] == logits[i] and j >= i) for j in range(lo, hi)):
            out.append(i)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    ckpt_dir = os.path.join(args.out, "checkpoint")
    os.makedirs(ckpt_dir, exist_ok=True)
    ckpt = os.path.join(ckpt_dir, "small0.ckpt")
    if not os.path.exists(ckpt):
        urllib.request.urlretrieve(CKPT_URL, ckpt)
    digest = sha256(ckpt)
    print(f"checkpoint small0.ckpt {os.path.getsize(ckpt)} bytes sha256 {digest}")

    import torchaudio
    from beat_this.inference import Audio2Frames
    from beat_this.model.beat_tracker import BeatThis

    state = torch.load(ckpt, map_location="cpu", weights_only=True)
    hp = {k: v for k, v in state["hyper_parameters"].items() if k in inspect.signature(BeatThis).parameters}
    model = BeatThis(**hp)
    model.load_state_dict({k[len("model.") :]: v for k, v in state["state_dict"].items() if k.startswith("model.")})
    model.eval()

    fbank = torchaudio.functional.melscale_fbanks(
        N_FFT // 2 + 1, 30.0, 11000.0, 128, SR, norm=None, mel_scale="slaney"
    )
    front_path = os.path.join(args.out, "beat_this_frontend.onnx")
    torch.onnx.export(
        Frontend(fbank).eval(),
        (torch.zeros(1, CHUNK),),
        front_path,
        input_names=["samples"],
        output_names=["spect"],
        dynamic_axes={"samples": {1: "n"}, "spect": {1: "frames"}},
        opset_version=17,
        dynamo=False,
    )
    onnx_path = os.path.join(args.out, "beat_this_small0.onnx")
    torch.onnx.export(
        Network(model).eval(),
        (torch.zeros(1, CHUNK_FRAMES, 128),),
        onnx_path,
        input_names=["spect"],
        output_names=["beat", "downbeat"],
        dynamic_axes={"spect": {1: "frames"}, "beat": {1: "frames"}, "downbeat": {1: "frames"}},
        opset_version=17,
        dynamo=False,
    )
    for p in (front_path, onnx_path):
        print(f"onnx {os.path.basename(p)} {os.path.getsize(p)} bytes sha256 {sha256(p)}")

    import onnxruntime as ort

    front = ort.InferenceSession(front_path, providers=["CPUExecutionProvider"])
    sess = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
    reference = Audio2Frames(checkpoint_path=ckpt, device="cpu", float16=False)
    fixtures = {
        "click_120_4_4": click_track(120, 4, 75.0, 1),
        "waltz_100_3_4": click_track(100, 3, 70.0, 2),
        "click_96_4_4_short": click_track(96, 4, 20.0, 3),
    }
    report = {
        "checkpointSha256": digest,
        "frontendSha256": sha256(front_path),
        "frontendBytes": os.path.getsize(front_path),
        "networkSha256": sha256(onnx_path),
        "networkBytes": os.path.getsize(onnx_path),
        "fixtures": {},
    }
    worst = 0.0
    beats_ok = True
    for name, x in fixtures.items():
        with torch.no_grad():
            rb, rd = reference(x, SR)
        rb, rd = rb.numpy(), rd.numpy()
        ob, od = ours(front, sess, x)
        n = min(len(rb), len(ob))
        diff = float(max(np.abs(rb[:n] - ob[:n]).max(), np.abs(rd[:n] - od[:n]).max()))
        pr, po = peaks(rb[:n]), peaks(ob[:n])
        same = len(pr) == len(po) and all(abs(a - b) <= 1 for a, b in zip(pr, po))
        pdr, pdo = peaks(rd[:n]), peaks(od[:n])
        same_d = len(pdr) == len(pdo) and all(abs(a - b) <= 1 for a, b in zip(pdr, pdo))
        worst = max(worst, diff)
        beats_ok = beats_ok and same and same_d
        ibi = np.diff(np.array(po) / 50.0)
        report["fixtures"][name] = {
            "frames": n,
            "maxLogitDiff": diff,
            "beatsRef": len(pr),
            "beatsOurs": len(po),
            "downbeatsRef": len(pdr),
            "downbeatsOurs": len(pdo),
            "beatsWithinOneFrame": same,
            "downbeatsWithinOneFrame": same_d,
            "medianIbiOurs": float(np.median(ibi)) if len(ibi) else None,
        }
        print(f"{name}: max logit diff {diff:.2e}, beats {len(pr)}/{len(po)} same={same}, downbeats {len(pdr)}/{len(pdo)} same={same_d}")
        np.asarray(x, "<f4").tofile(os.path.join(args.out, f"{name}.f32"))
        np.stack([ob[:n], od[:n]]).astype("<f4").tofile(os.path.join(args.out, f"{name}.logits.f32"))
    report["worstLogitDiff"] = worst
    report["pass"] = worst <= 1e-3 and beats_ok
    with open(os.path.join(args.out, "parity.json"), "w") as f:
        json.dump(report, f, indent=1)
    print("PASS" if report["pass"] else "FAIL", f"worst {worst:.2e}")
    sys.exit(0 if report["pass"] else 1)


if __name__ == "__main__":
    main()
