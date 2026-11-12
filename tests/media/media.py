#!/usr/bin/env python3
"""Real-media test matrix for change add-desktop-shell-and-native-preview (task 2.1).

The synthetic matrix is generated locally with the ffmpeg CLI (no media in git). Real camera footage is optional and
fetched from URLs listed in MANIFEST.json ("real" entries) when the owner provides them.

  python3 tests/media/media.py generate         # build tests/media/files/* and check sha256 against MANIFEST.json
  python3 tests/media/media.py verify           # checksum existing files only (strict)
  python3 tests/media/media.py update-manifest  # regenerate what differs and record new sha256 values
                                                # (--force regenerates everything, e.g. for another ffmpeg)
  python3 tests/media/media.py fetch            # download "real" entries that have a url (skips when none)

The generator is `OVE_FIXTURE_FFMPEG` if set, else `ffmpeg` on PATH (it needs lavfi sources and the encoders).

Safe to run from many test processes at once:
- `generate` holds an exclusive lock (`files/.lock`, fcntl.flock) for the whole run, so concurrent callers wait and
  then find the files ready;
- every file is written under a temporary name and renamed into place, so a reader never sees a partial file;
- truncated files are cut only from a complete source whose checksum was verified.

Encoders are not bit-reproducible across machines: different ffmpeg builds differ, and even the same build gives
different bytes with a different CPU or thread count. A file whose checksum differs from MANIFEST.json is regenerated
once; if the fresh file still differs, that is a warning: the file is checked for integrity with ffprobe and its
checksum recorded in `files/.generated.json`, so the next run accepts it without regenerating. A file that fails the
integrity check (short, unreadable) is an error. Set OVE_MEDIA_STRICT=1 on the machine that made the manifest to
require exact checksums there.
"""
import fcntl
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
LOCK = os.path.join(FILES, ".lock")
STAMPS = os.path.join(FILES, ".generated.json")

V = ["-f", "lavfi", "-i"]


def video(size="320x180", rate=30, dur=3):
    return V + [f"testsrc2=size={size}:rate={rate}:duration={dur}"]


def tone(sr=48000, dur=3):
    return V + [f"sine=frequency=440:sample_rate={sr}:duration={dur}"]




def patches(grey, skin, white):
    """Three vertical bands of known 10-bit limited-range code values: the BT.2408 18 % grey card, a skin tone
    (luma, Cb, Cr: SDR R'G'B' 0.78/0.55/0.45 carried into HDR through the reference OOTF) and HDR reference white.
    Tone-mapping tests check where each lands in SDR."""
    sy, scb, scr = skin
    return V + [
        f"color=c=black:s=1280x720:r=30:d=1,format=yuv420p10le,"
        f"geq=lum='if(lt(X\\,W/3)\\,{grey}\\,if(lt(X\\,2*W/3)\\,{sy}\\,{white}))'"
        f":cb='if(between(X\\,W/3\\,2*W/3-1)\\,{scb}\\,512)':cr='if(between(X\\,W/3\\,2*W/3-1)\\,{scr}\\,512)'"
    ]


def hevc_hdr(trc):
    return ["-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le", "-tag:v", "hvc1",
            "-color_primaries", "bt2020", "-color_trc", trc, "-colorspace", "bt2020nc",
            "-x265-params", f"log-level=error:colorprim=bt2020:transfer={trc}:colormatrix=bt2020nc"]


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
    "h264_4k60.mp4": (
        "H.264 High 3840x2160 60 fps (1 s, GOP 30) — 4K60 decode and seek at full size",
        "MP4 / H.264 4K60",
        video(size="3840x2160", rate=60, dur=1)
        + ["-c:v", "libx264", "-preset", "ultrafast", "-profile:v", "high", "-pix_fmt", "yuv420p", "-g", "30"],
    ),
    "hevc_hdr_vfr.mov": (
        "Phone-like HEVC Main10 HDR (BT.2020 PQ tags), variable frame rate, AAC",
        "MOV / HEVC HDR VFR",
        video(size="1280x720", dur=2) + tone(sr=44100, dur=2)
        + ["-vf", "settb=1/600,setpts=(N/30+0.012*mod(N\\,3))/TB", "-fps_mode", "passthrough",
           "-enc_time_base", "1/600", "-video_track_timescale", "600",
           "-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le", "-tag:v", "hvc1",
           "-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc",
           "-x265-params", "log-level=error:hdr10=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
           "-c:a", "aac", "-ac", "2", "-shortest"],
    ),
    "hevc_hlg_patches.mov": (
        "iPhone-like HEVC Main10 HLG (BT.2020, ARIB STD-B67): grey card 38 %, skin tone, reference white 75 %",
        "MOV / HEVC HLG",
        patches(397, (529, 462, 557), 721) + hevc_hdr("arib-std-b67"),
    ),
    "hevc_pq_patches.mov": (
        "HEVC Main10 PQ (BT.2020, SMPTE ST 2084): grey card 26 cd/m2, skin tone, reference white 203 cd/m2",
        "MOV / HEVC PQ",
        patches(401, (464, 488, 535), 573) + hevc_hdr("smpte2084"),
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

# Smallest plausible size of any generated file; anything shorter is a damaged leftover.
MIN_BYTES = 4096


class GenerateError(Exception):
    pass


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def ffmpeg_bin():
    return os.environ.get("OVE_FIXTURE_FFMPEG") or "ffmpeg"


def ffprobe_bin():
    b = ffmpeg_bin()
    d, base = os.path.split(b)
    return os.path.join(d, base.replace("ffmpeg", "ffprobe")) if "ffmpeg" in base else "ffprobe"


def ffmpeg_version():
    out = subprocess.run([ffmpeg_bin(), "-version"], capture_output=True, text=True, check=True).stdout
    return out.splitlines()[0]


def run(args):
    subprocess.run([ffmpeg_bin(), "-hide_banner", "-loglevel", "error", "-y"] + args, check=True)


def warn(msg):
    print(f"warning: {msg}", file=sys.stderr)


def tmp_name(name):
    # Same directory (atomic rename) and same extension (ffmpeg picks the muxer from it).
    return os.path.join(FILES, f".tmp-{os.getpid()}-{name}")


def atomic_write(name, build):
    """Runs `build(tmp_path)` and renames the result to `name` only when it succeeded."""
    tmp = tmp_name(name)
    try:
        build(tmp)
        with open(tmp, "rb") as f:
            os.fsync(f.fileno())
        os.replace(tmp, os.path.join(FILES, name))
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)


def integrity_problem(path):
    """None if the file looks complete (size, ffprobe reads a positive duration), else what is wrong."""
    size = os.path.getsize(path)
    if size < MIN_BYTES:
        return f"only {size} bytes"
    p = subprocess.run([ffprobe_bin(), "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                       capture_output=True, text=True)
    try:
        dur = float(p.stdout.strip())
    except ValueError:
        dur = 0.0
    if p.returncode != 0 or dur < 0.5:
        return f"unreadable (ffprobe: {p.stderr.strip() or p.stdout.strip() or 'no duration'})"
    return None


def load_manifest():
    with open(MANIFEST) as f:
        return json.load(f)


def load_stamps():
    try:
        with open(STAMPS) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_stamps(stamps):
    tmp = STAMPS + f".{os.getpid()}.tmp"
    with open(tmp, "w") as f:
        json.dump(stamps, f, indent=2, sort_keys=True)
    os.replace(tmp, STAMPS)


class Checker:
    """Decides whether a file on disk is good: manifest checksum, or a checksum this machine recorded after an
    integrity check with this same ffmpeg."""

    def __init__(self, manifest, version, recording=False):
        # Recording (update-manifest): fresh files only need to pass the integrity check.
        self.recording = recording
        self.expected = {e["name"]: e["sha256"] for e in manifest["synthetic"]}
        self.manifest_version = manifest["generatedWith"]
        self.version = version
        self.strict = os.environ.get("OVE_MEDIA_STRICT") == "1"
        self.stamps = load_stamps()

    def good(self, name):
        path = os.path.join(FILES, name)
        if not os.path.exists(path):
            return False
        got = sha256(path)
        want = self.expected.get(name)
        if got == want:
            return True
        if want and self.strict:
            return False  # strict mode: the manifest checksum must be reproduced exactly
        s = self.stamps.get(name)
        return bool(s) and s.get("sha256") == got and s.get("ffmpeg") == self.version

    def accept_fresh(self, name):
        """Called after (re)generating `name`; raises GenerateError if the new file is not acceptable."""
        path = os.path.join(FILES, name)
        problem = integrity_problem(path) if not name.startswith("truncated_") else None
        if problem:
            raise GenerateError(f"{name}: freshly generated file is damaged: {problem}")
        got = sha256(path)
        want = self.expected.get(name)
        if got == want or self.recording:
            self.stamps.pop(name, None)
            return
        if want and self.strict:
            raise GenerateError(f"{name}: checksum {got} != manifest {want} (OVE_MEDIA_STRICT=1, ffmpeg {self.version})")
        if want:
            warn(f"{name}: checksum differs from MANIFEST.json (made with {self.manifest_version}; this is "
                 f"{self.version}); accepted after an integrity check. Run update-manifest to record it.")
        else:
            warn(f"{name}: not in MANIFEST.json yet; accepted after an integrity check. Run update-manifest.")
        self.stamps[name] = {"sha256": got, "ffmpeg": self.version}


def derive_truncated(name, src, frac):
    """Remuxes `src` and keeps the first `frac` of its bytes, written atomically."""
    full = tmp_name("full-" + name)
    flags = ["-movflags", "+faststart"] if "faststart" in name else []
    try:
        run(["-i", os.path.join(FILES, src), "-c", "copy", "-fflags", "+bitexact"] + flags + [full])
        if integrity_problem(full):
            raise GenerateError(f"{name}: remux of {src} is damaged")
        with open(full, "rb") as f:
            data = f.read()
    finally:
        if os.path.exists(full):
            os.remove(full)
    keep = int(len(data) * frac)

    def build(tmp):
        with open(tmp, "wb") as f:
            f.write(data[:keep])
    atomic_write(name, build)
    if os.path.getsize(os.path.join(FILES, name)) != keep or keep < MIN_BYTES:
        raise GenerateError(f"{name}: expected {keep} bytes")


def generate(force=False, recording=False):
    os.makedirs(FILES, exist_ok=True)
    with open(LOCK, "a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            checker = Checker(load_manifest(), ffmpeg_version(), recording)
            regenerated = set()
            for name, (_, _, args) in MATRIX.items():
                out = os.path.join(FILES, name)
                if not force and checker.good(name):
                    continue
                if os.path.exists(out) and not force:
                    warn(f"regenerating {name}: checksum differs from MANIFEST.json and from this machine's record")
                atomic_write(name, lambda tmp, a=args: run(
                    a + ["-fflags", "+bitexact", "-flags:v", "+bitexact", "-flags:a", "+bitexact", tmp]))
                checker.accept_fresh(name)
                regenerated.add(name)
            for name, (_, src, frac) in TRUNCATED.items():
                if not force and src not in regenerated and checker.good(name):
                    continue
                if not checker.good(src):
                    raise GenerateError(f"{name}: source {src} is not verified")
                if os.path.exists(os.path.join(FILES, name)) and src not in regenerated and not force:
                    warn(f"regenerating {name}: checksum differs from MANIFEST.json and from this machine's record")
                derive_truncated(name, src, frac)
                checker.accept_fresh(name)
            save_stamps(checker.stamps)
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


def entries():
    for name, (desc, fmt, _) in MATRIX.items():
        yield name, desc, fmt
    for name, (desc, _, _) in TRUNCATED.items():
        yield name, desc, "truncated"


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


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "generate"
    try:
        if cmd == "generate":
            generate()
            print(f"{len(MATRIX) + len(TRUNCATED)} synthetic files ready in {FILES}")
        elif cmd == "verify":
            verify(strict=True)
        elif cmd == "update-manifest":
            generate(force="--force" in sys.argv, recording=True)
            update_manifest()
        elif cmd == "fetch":
            fetch()
        else:
            sys.exit(__doc__)
    except GenerateError as e:
        sys.exit(f"media generation failed: {e}")
    except (OSError, subprocess.CalledProcessError) as e:
        sys.exit(f"media generation failed: {e}")


if __name__ == "__main__":
    main()
