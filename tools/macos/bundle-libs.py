#!/usr/bin/env python3
"""Make the FFmpeg libraries in LIBDIR self-contained for Contents/Frameworks of a macOS app.

    tools/macos/bundle-libs.py LIBDIR        # prepare (tools/package.sh, before building)
    tools/macos/bundle-libs.py --check APP   # verify a built .app

Prepare: every non-system library FFmpeg loads (dav1d from Homebrew) is copied into LIBDIR; every reference between
them becomes @rpath/<real file name> (the files are shipped under their real names, without the soname symlinks);
each library's id becomes @rpath/<its name>; and every changed library is signed again ad hoc (Apple silicon refuses
to load a library whose signature install_name_tool invalidated). Idempotent.

Check: every Mach-O file in Contents/MacOS and Contents/Frameworks loads only system libraries or @rpath libraries that
are in Contents/Frameworks, and the code signature of each library is valid.
"""
import os
import re
import shutil
import subprocess
import sys

SYSTEM = ("/usr/lib/", "/System/")
FFMPEG = re.compile(r"^lib(avcodec|avformat|avutil|swscale|swresample)\.")


def run(*args: str) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def own_id(path: str) -> str | None:
    lines = run("otool", "-D", path).splitlines()
    return lines[1].strip() if len(lines) > 1 else None


def deps(path: str) -> list[str]:
    me = own_id(path)
    out = []
    for line in run("otool", "-L", path).splitlines()[1:]:
        dep = line.strip().split(" (")[0]
        if dep and dep != me:
            out.append(dep)
    return out


def prepare(lib: str) -> None:
    queue = sorted(
        os.path.join(lib, n) for n in os.listdir(lib) if FFMPEG.match(n) and n.endswith(".dylib")
        and not os.path.islink(os.path.join(lib, n))
    )
    done: set[str] = set()
    while queue:
        f = queue.pop()
        if f in done:
            continue
        done.add(f)
        changed = False
        for dep in deps(f):
            if dep.startswith(SYSTEM):
                continue
            name = os.path.basename(dep)
            if dep.startswith("@rpath/") or os.path.dirname(os.path.realpath(dep)) == os.path.realpath(lib):
                src = os.path.join(lib, name)
            else:
                src = dep
            if not os.path.exists(src):
                sys.exit(f"{os.path.basename(f)} loads {dep}, which is not on this machine")
            real = os.path.realpath(src)
            target = os.path.join(lib, os.path.basename(real))
            if not os.path.exists(target):
                shutil.copy2(real, target)
                os.chmod(target, 0o755)
                print(f"bundled {real}")
            queue.append(target)
            new = "@rpath/" + os.path.basename(real)
            if dep != new:
                run("install_name_tool", "-change", dep, new, f)
                changed = True
        new_id = "@rpath/" + os.path.basename(f)
        if own_id(f) != new_id:
            run("install_name_tool", "-id", new_id, f)
            changed = True
        if changed:
            run("codesign", "--force", "--sign", "-", f)
    print(f"{len(done)} libraries ready in {lib}")


def is_macho(path: str) -> bool:
    with open(path, "rb") as fh:
        return fh.read(4) in (b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca")


def check(app: str) -> None:
    fw = os.path.join(app, "Contents", "Frameworks")
    shipped = set(os.listdir(fw)) if os.path.isdir(fw) else set()
    files = [os.path.join(app, "Contents", "MacOS", n) for n in os.listdir(os.path.join(app, "Contents", "MacOS"))]
    files += [os.path.join(fw, n) for n in sorted(shipped)]
    bad = []
    for f in files:
        if not os.path.isfile(f) or not is_macho(f):
            continue
        for dep in deps(f):
            if dep.startswith(SYSTEM):
                continue
            if dep.startswith("@rpath/") and dep[len("@rpath/"):] in shipped:
                continue
            bad.append(f"{os.path.basename(f)} → {dep}")
        if f.startswith(fw):
            r = subprocess.run(["codesign", "--verify", "--strict", f], capture_output=True, text=True)
            if r.returncode != 0:
                bad.append(f"{os.path.basename(f)}: invalid code signature: {r.stderr.strip()}")
    if bad:
        sys.exit("The app would not load on another Mac:\n  " + "\n  ".join(bad))
    print(f"library check passed: {len(files)} files in {os.path.basename(app)} load only shipped or system libraries")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--check":
        check(sys.argv[2])
    elif len(sys.argv) == 2:
        prepare(sys.argv[1])
    else:
        sys.exit(__doc__)
