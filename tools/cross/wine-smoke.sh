#!/usr/bin/env bash
# Smoke test of the Windows `ove.exe` under Wine on Linux (cross builds, see tools/package.sh --target windows-x64-gnu).
#
#   tools/cross/wine-smoke.sh DIR [MEDIA_DIR]
#
# DIR holds ove.exe and the FFmpeg DLLs, as installed (an extracted -setup.exe works: `7z x -oDIR setup.exe`).
# MEDIA_DIR defaults to tests/media/files (`python3 tests/media/media.py generate`). Runs --version, doctor, project
# create, media add (H.264, HEVC 10-bit, AV1 through dav1d, ProRes, VP9, AAC), media list/probe, frame grabs (AV1,
# HEVC, ProRes), a composited contact sheet, source print, verify,
# export to MP4 and timeline export to FCPXML in a throwaway Wine prefix, and checks the outputs with ffprobe when it
# is on PATH. Exits non-zero on the first failure.
#
# Passing under Wine shows the binaries load, find their DLLs and run the document, media and export code paths. It
# does not prove they work on Windows: Wine is not Windows (no Media Foundation encoders, D3D11VA or WebView2 here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="$(cd "${1:?usage: $0 DIR [MEDIA_DIR]}" && pwd)"
MEDIA="$(cd "${2:-$ROOT/tests/media/files}" && pwd)"
[ -f "$DIR/ove.exe" ] || { echo "no ove.exe in $DIR" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'wineserver -k >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT
export WINEPREFIX="$WORK/prefix" WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
# Only the folder of ove.exe supplies DLLs (no WINEPATH), as on an installed machine.
unset WINEPATH DISPLAY
wineboot -i >/dev/null 2>&1 || true

win() { winepath -w "$1" 2>/dev/null; }
LIB="$(win "$WORK/lib")"
mkdir -p "$WORK/lib"
P="Wine smoke"
step() { echo; echo "== ove $*"; }
ove() { step "$@"; wine "$DIR/ove.exe" --library "$LIB" "$@"; }

step --version
wine "$DIR/ove.exe" --version
ove doctor
ove project create "$P"
files=()
for f in h264_longgop.mp4 hevc_main10_420.mp4 av1.mkv prores_hq.mov vp9_opus.webm audio_aac.m4a; do
  [ -f "$MEDIA/$f" ] || { echo "missing test media $MEDIA/$f (python3 tests/media/media.py generate)" >&2; exit 2; }
  files+=("$(win "$MEDIA/$f")")
done
ove media add "$P" "${files[@]}"
ove media list "$P"
ove media probe "$P" av1.mkv
for f in av1.mkv hevc_main10_420.mp4 prores_hq.mov; do
  ove media grab "$P" "$f" --at 0.5 --width 320 --out "$(win "$WORK/grab-$f.png")"
  [ -s "$WORK/grab-$f.png" ] || { echo "no frame decoded from $f" >&2; exit 1; }
done
ove look "$P" --count 4 --width 320 --sheet "$(win "$WORK/sheet.png")"
ove source print "$P"
ove verify "$P"
ove export "$P" "$(win "$WORK/out.mp4")" --height 360 --fps 25 2>&1 | tee "$WORK/export.log"
ove timeline export "$P" "$(win "$WORK/out.fcpxml")"

echo
echo "== outputs"
[ -s "$WORK/out.mp4" ] || { echo "export wrote no MP4" >&2; exit 1; }
if command -v ffprobe >/dev/null; then
  ffprobe -v error -show_entries stream=codec_name,width,height -show_entries format=duration -of compact "$WORK/out.mp4"
fi
grep -q "<fcpxml" "$WORK/out.fcpxml" || { echo "timeline export wrote no FCPXML" >&2; exit 1; }
python3 -c "import sys, xml.dom.minidom as m; m.parse(sys.argv[1])" "$WORK/out.fcpxml"
echo "out.mp4 $(wc -c < "$WORK/out.mp4") bytes; out.fcpxml $(wc -c < "$WORK/out.fcpxml") bytes, well-formed"
echo "Wine smoke test passed"
