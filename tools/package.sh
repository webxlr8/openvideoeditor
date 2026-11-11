#!/usr/bin/env bash
# Builds installable app bundles for this host (roadmap change 16, unsigned v1):
#   macOS arm64 → .app + .dmg      Windows x64 → .msi + NSIS .exe      Linux x64 (dev) → .deb
#
#   tools/package.sh [--skip-ffmpeg] [--debug]
#
# Steps: LGPL FFmpeg build (tools/ffmpeg/build.sh) → generated bundle config that ships exactly the five LGPL
# libraries (+ dav1d) → the `ove` CLI as a sidecar → `tauri build` → artefact list with sizes against the 60 MB
# installer budget.
# Signing and notarisation (Apple Developer ID, Windows code-signing certificate) come later; see docs/03 §6.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SKIP_FFMPEG=0
PROFILE=(--)
for a in "$@"; do
  case "$a" in
    --skip-ffmpeg) SKIP_FFMPEG=1 ;;
    --debug) PROFILE=(--debug --) ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) TARGET=macos-arm64 ;;
  MINGW64_NT*|MSYS_NT*) TARGET=windows-x64 ;;
  Linux-x86_64) TARGET=linux-x64 ;;
  *) echo "unsupported host" >&2; exit 2 ;;
esac

# Linux packages are development builds that depend on the distribution's FFmpeg (see below), so they link it.
[ "$TARGET" = linux-x64 ] && export OVE_USE_SYSTEM_FFMPEG=1 && SKIP_FFMPEG=1
[ "$SKIP_FFMPEG" = 1 ] || "$ROOT/tools/ffmpeg/build.sh"
# shellcheck disable=SC1091
source "$ROOT/tools/ffmpeg/env.sh"
LIB="$ROOT/tools/ffmpeg/out/$TARGET"
GEN="$ROOT/apps/desktop/src-tauri/tauri.bundle.generated.json"

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
  windows-x64)
    python3 - "$GEN" <<PY
import json, sys
libs = """$(shipped)""".split()
json.dump({"bundle": {"resources": {l: "./" for l in libs}, "targets": ["msi", "nsis"]}}, open(sys.argv[1], "w"), indent=2)
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
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
EXE=""; [ "$TARGET" = windows-x64 ] && EXE=".exe"
CLI_PROFILE=release; [ "${PROFILE[0]}" = --debug ] && CLI_PROFILE=debug
( cd "$ROOT" && cargo build -p ove-cli --features media $([ "$CLI_PROFILE" = release ] && echo --release) )
mkdir -p "$ROOT/apps/desktop/src-tauri/binaries"
cp "${CARGO_TARGET_DIR:-$ROOT/target}/$CLI_PROFILE/ove$EXE" "$ROOT/apps/desktop/src-tauri/binaries/ove-$TRIPLE$EXE"
python3 - "$GEN" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["bundle"]["externalBin"] = ["binaries/ove"]
json.dump(c, open(sys.argv[1], "w"), indent=2)
PY

cd "$ROOT/apps/desktop"
pnpm install --frozen-lockfile
pnpm tauri build --config "$GEN" "${PROFILE[@]}"

BUNDLE_DIR="${CARGO_TARGET_DIR:-$ROOT/target}/$([ "${PROFILE[0]}" = --debug ] && echo debug || echo release)/bundle"
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
