#!/usr/bin/env python3
"""Real-media test matrix for change add-desktop-shell-and-native-preview (task 2.1).

The synthetic matrix is generated locally with the ffmpeg CLI (no media in git). Real camera footage is optional and
fetched from URLs listed in MANIFEST.json ("real" entries) when the owner provides them.

  python3 tests/media/media.py generate         # build tests/media/files/* and check sha256 against MANIFEST.json
  python3 tests/media/media.py verify           # checksum existing files only (strict)
  python3 tests/media/media.py update-manifest  # regenerate and record new sha256 values (other ffmpeg builds)
  python3 tests/media/media.py fetch            # download "real" entries that have a url (skips when none)

Encoders are not bit-reproducible across ffmpeg builds, so a checksum mismatch on another machine is reported as a
warning together with the ffmpeg version recorded in the manifest; run update-manifest there.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
FILES = os.path.join(HERE, "files")
MANIFEST = os.path.join(HERE, "MANIFEST.json")

V = ["-f", "lavfi", "-i"]


def video(size="320x180", rate=30, dur=3):
    return V + [f"testsrc2=size={size}:rate={rate}:duration={dur}"]


def tone(sr=48000, dur=3):
    return V + [f"sine=frequency=440:sample_rate={sr}:duration={dur}"]


# name -> (description, spec format, ffmpeg args)
MATRIX = {
    "h264_longgop.mp4": (
        "H.264 High long-GOP (GOP 60, 3 B-frames) 30 fps with AAC stereo",
        "MP4 / H.264",
        video(dur=4) + tone(dur=4)
        + ["-c:v", "libx264", "-profile:v", "high", "-pix_fmt", "yuv420p", "-g", "60", "-bf", "3",
           "-c:a", "aac", "-ac", "2", "-shortest"],
    ),
    "h264_60fps.mov": (
        "H.264 Main 60 fps in MOV, no audio",
        "MOV / H.264 60p",
        video(rate=60, dur=2) + ["-c:v", "libx264", "-profile:v", "main", "-pix_fmt", "yuv420p", "-g", "120"],
    ),
    "hevc_main10_420.mp4": (
        "HEVC Main10 4:2:0 10-bit, hvc1 tag",
        "MP4 / HEVC 10-bit 4:2:0",
        video() + ["-c:v", "libx265", "-pix_fmt", "yuv420p10le", "-tag:v", "hvc1", "-x265-params", "log-level=error"],
    ),
    "hevc_main422_10.mov": (
        "HEVC Main 4:2:2 10-bit (not hardware-decodable on many GPUs)",
        "MOV / HEVC 10-bit 4:2:2",
        video() + ["-c:v", "libx265", "-pix_fmt", "yuv422p10le", "-x265-params", "log-level=error"],
    ),
    "prores_proxy.mov": ("ProRes 422 Proxy", "MOV / ProRes 422 Proxy",
                         video() + ["-c:v", "prores_ks", "-profile:v", "0", "-pix_fmt", "yuv422p10le"]),
    "prores_lt.mov": ("ProRes 422 LT", "MOV / ProRes 422 LT",
                      video() + ["-c:v", "prores_ks", "-profile:v", "1", "-pix_fmt", "yuv422p10le"]),
    "prores_hq.mov": (
        "ProRes 422 HQ with 24-bit PCM audio",
        "MOV / ProRes 422 HQ",
        video() + tone() + ["-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le",
                            "-c:a", "pcm_s24le", "-ac", "2"],
    ),
    "prores_4444.mov": ("ProRes 4444 with alpha", "MOV / ProRes 4444",
                        video() + ["-c:v", "prores_ks", "-profile:v", "4", "-pix_fmt", "yuva444p10le"]),
    "dnxhr_hq.mxf": (
        "DNxHR HQ 4:2:2 8-bit in MXF OP1a, 25 fps, PCM audio",
        "MXF / DNxHR HQ",
        video(size="1280x720", rate=25, dur=2) + tone(dur=2)
        + ["-c:v", "dnxhd", "-profile:v", "dnxhr_hq", "-pix_fmt", "yuv422p", "-c:a", "pcm_s16le", "-ac", "1"],
    ),
    "av1.mkv": ("AV1 in Matroska", "MKV / AV1",
                video() + ["-c:v", "libsvtav1", "-preset", "12", "-pix_fmt", "yuv420p", "-g", "60"]),
    "vp9_opus.webm": (
        "VP9 with Opus audio",
        "WebM / VP9 + Opus",
        video() + tone() + ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "500k",
                            "-c:a", "libopus", "-ac", "2"],
    ),
    "vfr_phone.mp4": (
        "Variable frame rate (irregular timestamps, phone-like 600 timescale) H.264 with AAC",
        "MP4 / H.264 VFR",
        video(dur=4) + tone(sr=44100, dur=4)
        + ["-vf", "settb=1/600,setpts=(N/30+0.012*mod(N\\,3))/TB", "-fps_mode", "passthrough",
           "-enc_time_base", "1/600", "-video_track_timescale", "600",
           "-c:v", "libx264", "-pix_fmt", "yuv420p", "-bf", "0", "-c:a", "aac", "-ac", "2", "-shortest"],
    ),
    "audio_aac.m4a": ("AAC-LC stereo 44.1 kHz", "M4A / AAC", tone(sr=44100) + ["-c:a", "aac", "-ac", "2"]),
    "audio_pcm.wav": ("PCM s16 stereo 44.1 kHz", "WAV / PCM", tone(sr=44100) + ["-c:a", "pcm_s16le", "-ac", "2"]),
    "audio_alac.m4a": ("ALAC stereo 48 kHz", "M4A / ALAC", tone() + ["-c:a", "alac", "-ac", "2"]),
    "audio_opus.mka": ("Opus stereo 48 kHz", "MKA / Opus", tone() + ["-c:a", "libopus", "-ac", "2"]),
}

# Derived files: name -> (description, source, fraction of bytes kept)
TRUNCATED = {
    "truncated_faststart.mp4": ("H.264 with moov at front, cut at 45% (opens, ends early)", "h264_longgop.mp4", 0.45),
    "truncated_nomoov.mp4": ("H.264 with moov at end, cut at 50% (cannot be opened)", "h264_longgop.mp4", 0.50),
}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def ffmpeg_version():
    out = subprocess.run(["ffmpeg", "-version"], capture_output=True, text=True, check=True).stdout
    return out.splitlines()[0]


def run(args):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"] + args, check=True)


def generate():
    os.makedirs(FILES, exist_ok=True)
    for name, (_, _, args) in MATRIX.items():
        out = os.path.join(FILES, name)
        if not os.path.exists(out):
            run(args + ["-fflags", "+bitexact", "-flags:v", "+bitexact", "-flags:a", "+bitexact", out])
    for name, (_, src, frac) in TRUNCATED.items():
        out = os.path.join(FILES, name)
        if os.path.exists(out):
            continue
        tmp = out + ".full.mp4"
        flags = ["-movflags", "+faststart"] if "faststart" in name else []
        run(["-i", os.path.join(FILES, src), "-c", "copy", "-fflags", "+bitexact"] + flags + [tmp])
        with open(tmp, "rb") as f:
            data = f.read()
        with open(out, "wb") as f:
            f.write(data[: int(len(data) * frac)])
        os.remove(tmp)


def entries():
    for name, (desc, fmt, _) in MATRIX.items():
        yield name, desc, fmt
    for name, (desc, _, _) in TRUNCATED.items():
        yield name, desc, "truncated"


def load_manifest():
    with open(MANIFEST) as f:
        return json.load(f)


def verify(strict):
    m = load_manifest()
    bad = 0
    for e in m["synthetic"]:
        path = os.path.join(FILES, e["name"])
        if not os.path.exists(path):
            print(f"MISSING {e['name']}")
            bad += 1
            continue
        got = sha256(path)
        if got != e["sha256"]:
            print(f"CHECKSUM {e['name']}: {got} != {e['sha256']} (manifest made with {m['generatedWith']})")
            bad += 1
    for e in m.get("real", []):
        path = os.path.join(FILES, e["name"])
        if os.path.exists(path) and sha256(path) != e["sha256"]:
            print(f"CHECKSUM {e['name']} (real footage)")
            bad += 1
    print(f"{len(m['synthetic'])} synthetic files checked, {bad} problems")
    if bad and (strict or m["generatedWith"] == ffmpeg_version()):
        sys.exit(1)


def update_manifest():
    old = load_manifest() if os.path.exists(MANIFEST) else {}
    m = {
        "about": "Media matrix for decode/playback tests. Files live in tests/media/files (gitignored); "
                 "generate with `python3 tests/media/media.py generate`. 'real' entries are owner-provided "
                 "camera originals: {name, description, format, url, sha256}.",
        "generatedWith": ffmpeg_version(),
        "synthetic": [
            {"name": n, "description": d, "format": f, "sha256": sha256(os.path.join(FILES, n))}
            for n, d, f in entries()
        ],
        "real": old.get("real", []),
    }
    with open(MANIFEST, "w") as f:
        json.dump(m, f, indent=2)
        f.write("\n")


def fetch():
    m = load_manifest()
    os.makedirs(FILES, exist_ok=True)
    real = [e for e in m.get("real", []) if e.get("url")]
    if not real:
        print("no real-footage entries with a url; skipping")
    for e in real:
        path = os.path.join(FILES, e["name"])
        if not os.path.exists(path):
            print(f"fetching {e['name']}")
            with urllib.request.urlopen(e["url"]) as r, open(path + ".part", "wb") as f:
                shutil.copyfileobj(r, f)
            os.replace(path + ".part", path)
        if sha256(path) != e["sha256"]:
            sys.exit(f"checksum mismatch for {e['name']}")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "generate"
    if cmd == "generate":
        generate()
        verify(strict=False)
    elif cmd == "verify":
        verify(strict=True)
    elif cmd == "update-manifest":
        generate()
        update_manifest()
    elif cmd == "fetch":
        fetch()
    else:
        sys.exit(__doc__)
