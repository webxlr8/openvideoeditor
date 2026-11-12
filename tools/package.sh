#!/usr/bin/env bash
# Builds installable app bundles for this host (roadmap change 16, unsigned v1):
#   macOS arm64 → .app + .dmg      Windows x64 → .msi + NSIS .exe      Linux x64 (dev) → .deb
#   Linux x64 host with --target windows-x64-gnu → NSIS -setup.exe for Windows x64 (cross build with MinGW-w64; no
#   .msi, which needs WiX on Windows)
#
#   tools/package.sh [--skip-ffmpeg] [--debug] [--target windows-x64-gnu]
#
# Steps: LGPL FFmpeg build (tools/ffmpeg/build.sh) → generated bundle config that ships exactly the five LGPL
# libraries (+ dav1d) → the `ove` CLI as a sidecar → `tauri build` → artefact list with sizes against the 60 MB
# installer budget. Cross builds also check that every DLL the executables and libraries import is either shipped or
# part of Windows.
# Signing and notarisation (Apple Developer ID, Windows code-signing certificate) come later; see docs/03 §6.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SKIP_FFMPEG=0
PROFILE=(--)
TARGET=""
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-ffmpeg) SKIP_FFMPEG=1 ;;
    --debug) PROFILE=(--debug --) ;;
    --target) TARGET="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) HOST=macos-arm64 ;;
  MINGW64_NT*|MSYS_NT*) HOST=windows-x64 ;;
  Linux-x86_64) HOST=linux-x64 ;;
  *) echo "unsupported host" >&2; exit 2 ;;
esac
TARGET="${TARGET:-$HOST}"
case "$HOST/$TARGET" in
  "$TARGET/$TARGET") TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"; CROSS="" ;;
  linux-x64/windows-x64-gnu) TRIPLE=x86_64-pc-windows-gnu; CROSS="$TRIPLE" ;;
  *) echo "cannot package $TARGET on $HOST (cross builds: linux-x64 → windows-x64-gnu)" >&2; exit 2 ;;
esac

# Linux packages are development builds that depend on the distribution's FFmpeg (see below), so they link it.
[ "$TARGET" = linux-x64 ] && export OVE_USE_SYSTEM_FFMPEG=1 && SKIP_FFMPEG=1
[ "$SKIP_FFMPEG" = 1 ] || "$ROOT/tools/ffmpeg/build.sh" --target "$TARGET"
export OVE_FFMPEG_TARGET="$TARGET"
# shellcheck disable=SC1091
source "$ROOT/tools/ffmpeg/env.sh"
LIB="$ROOT/tools/ffmpeg/out/$TARGET"
GEN="$ROOT/apps/desktop/src-tauri/tauri.bundle.generated.json"
CLI_PROFILE=release; [ "${PROFILE[0]}" = --debug ] && CLI_PROFILE=debug
OUT_DIR="${CARGO_TARGET_DIR:-$ROOT/target}${CROSS:+/$CROSS}/$CLI_PROFILE"
# `--target <triple>` for cargo and tauri on cross builds, nothing otherwise.
TARGET_ARGS=(); [ -z "$CROSS" ] || TARGET_ARGS=(--target "$CROSS")

shipped() {
  # The shipped FFmpeg libraries for this target (real files, not symlinks; never libavfilter or the CLI).
  find "$LIB/lib" "$LIB/bin" -maxdepth 1 -type f \( -name '*.dylib' -o -name '*.dll' -o -name '*.so.*' \) 2>/dev/null \
    | grep -E 'avcodec|avformat|avutil|swscale|swresample|dav1d' | sort
}

case "$TARGET" in
  macos-arm64)
    # Install names → @rpath so the libraries load from Contents/Frameworks.
    while read -r f; do
      install_name_tool -id "@rpath/$(basename "$f")" "$f"
      for dep in $(otool -L "$f" | awk 'NR>1 {print $1}' | grep "$LIB/lib"); do
        install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$f"
      done
    done < <(shipped)
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"
    python3 - "$GEN" <<PY
import json, sys
libs = """$(shipped)""".split()
json.dump({"bundle": {"macOS": {"frameworks": libs, "minimumSystemVersion": "14.0"}, "targets": ["app", "dmg"]}},
          open(sys.argv[1], "w"), indent=2)
PY
    ;;
  windows-x64|windows-x64-gnu)
    # The DLLs go next to the executables (the install folder), where Windows looks first.
    python3 - "$GEN" "$TARGET" <<PY
import json, sys
libs = """$(shipped)""".split()
targets = ["nsis"] if sys.argv[2] == "windows-x64-gnu" else ["msi", "nsis"]
json.dump({"bundle": {"resources": {l: "./" for l in libs}, "targets": targets}}, open(sys.argv[1], "w"), indent=2)
PY
    ;;
  linux-x64)
    # Development packages only (Linux is not a v1 platform): depend on the distribution's FFmpeg instead of shipping it.
    python3 - "$GEN" <<'PY'
import json, sys
json.dump({"bundle": {"targets": ["deb"], "linux": {"deb": {"depends": [
    "libavcodec60 | libavcodec61 | libavcodec62", "libavformat60 | libavformat61 | libavformat62",
    "libavutil58 | libavutil59 | libavutil60", "libswscale7 | libswscale8 | libswscale9",
    "libswresample4 | libswresample5 | libswresample6", "libwebkit2gtk-4.1-0", "libgtk-3-0"]}}}},
    open(sys.argv[1], "w"), indent=2)
PY
    ;;
esac

# The `ove` command line (with media support) ships inside the app as a sidecar: next to the app executable on
# macOS / Windows, /usr/bin/ove in the .deb. Agents and `ove mcp` use it; the app's control link serves it.
EXE=""; case "$TARGET" in windows-*) EXE=".exe" ;; esac
( cd "$ROOT" && cargo build ${TARGET_ARGS[@]+"${TARGET_ARGS[@]}"} -p ove-cli --features media $([ "$CLI_PROFILE" = release ] && echo --release) )
mkdir -p "$ROOT/apps/desktop/src-tauri/binaries"
cp "$OUT_DIR/ove$EXE" "$ROOT/apps/desktop/src-tauri/binaries/ove-$TRIPLE$EXE"
python3 - "$GEN" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["bundle"]["externalBin"] = ["binaries/ove"]
json.dump(c, open(sys.argv[1], "w"), indent=2)
PY

cd "$ROOT/apps/desktop"
pnpm install --frozen-lockfile
pnpm tauri build --config "$GEN" ${TARGET_ARGS[@]+"${TARGET_ARGS[@]}"} "${PROFILE[@]}"

if [ "$TARGET" = windows-x64-gnu ]; then
  # Every DLL that the shipped executables and libraries import must be shipped too or come with Windows. The
  # MinGW runtime (libgcc, libstdc++, winpthreads) is linked statically, so nothing of it should show up here.
  # WebView2Loader.dll is installed next to the app by the NSIS installer of a -gnu build.
  python3 - "$OUT_DIR" "$(shipped)" <<'PY'
import os, re, subprocess, sys
out_dir, libs = sys.argv[1], sys.argv[2].split()
system = set("""advapi32 bcrypt bcryptprimitives combase comctl32 comdlg32 crypt32 d3d11 d3d12 d3dcompiler_47 dbghelp
    dwmapi dwrite dxgi gdi32 imm32 kernel32 mf mfplat mfreadwrite mmdevapi msvcrt ncrypt ntdll ole32 oleaut32 opengl32
    propsys psapi secur32 setupapi shell32 shlwapi user32 userenv uxtheme version winmm ws2_32""".split())
exes = [os.path.join(out_dir, n) for n in ("ove-desktop.exe", "ove.exe", "WebView2Loader.dll")]
shipped = {os.path.basename(p).lower() for p in libs + exes}
bad = []
for f in exes + libs:
    dump = subprocess.run(["x86_64-w64-mingw32-objdump", "-p", f], check=True, capture_output=True, text=True).stdout
    for dll in sorted({d.lower() for d in re.findall(r"DLL Name: (\S+)", dump)}):
        if dll in shipped or dll.removesuffix(".dll") in system or dll.startswith(("api-ms-win-", "ext-ms-win-")):
            continue
        bad.append(f"{os.path.basename(f)} → {dll}")
if bad:
    sys.exit("DLLs imported but neither shipped nor part of Windows:\n  " + "\n  ".join(bad))
print("DLL check passed: every import is shipped or a Windows system DLL")
PY
fi

BUNDLE_DIR="$OUT_DIR/bundle"
echo
echo "Artefacts:"
status=0
while read -r f; do
  size=$(wc -c < "$f" | tr -d ' ')
  mb=$(awk "BEGIN { printf \"%.1f\", $size / 1048576 }")
  over=""
  if [ "$size" -gt $((60 * 1048576)) ]; then over="  ← over the 60 MB budget"; status=1; fi
  echo "  $mb MB  $f$over"
done < <(find "$BUNDLE_DIR" -maxdepth 2 -type f \( -name '*.dmg' -o -name '*.msi' -o -name '*.exe' -o -name '*.deb' -o -name '*.AppImage' \))
exit $status
