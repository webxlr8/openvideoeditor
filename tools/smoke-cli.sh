#!/usr/bin/env bash
# Smoke test of an installed `ove` command line on the machine it was built for: macOS (inside the .app), Windows
# (installed by the setup.exe; run from Git Bash) or Linux. The same steps as tools/cross/wine-smoke.sh, natively.
#
#   tools/smoke-cli.sh PATH_TO_OVE [MEDIA_DIR]
#
# MEDIA_DIR defaults to tests/media/files (`python3 tests/media/media.py generate`). Runs --version, doctor, project
# create, media add (H.264, HEVC 10-bit, AV1, ProRes, VP9, AAC), media list/probe, frame grabs, a composited contact
# sheet, source print, verify, export to MP4 and timeline export to FCPXML in a throwaway library, and checks the
# outputs. Exits non-zero on the first failure.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OVE="${1:?usage: $0 PATH_TO_OVE [MEDIA_DIR]}"
MEDIA="$(cd "${2:-$ROOT/tests/media/files}" && pwd)"
[ -x "$OVE" ] || { echo "not executable: $OVE" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
# Native path for the program: Windows paths under Git Bash, unchanged elsewhere.
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
mkdir -p "$WORK/lib"
LIB="$(native "$WORK/lib")"
P="Smoke test"
step() { echo; echo "== ove $*"; }
ove() { step "$@"; "$OVE" --library "$LIB" "$@"; }

step --version
"$OVE" --version
ove doctor
ove project create "$P"
files=()
for f in h264_longgop.mp4 hevc_main10_420.mp4 av1.mkv prores_hq.mov vp9_opus.webm audio_aac.m4a; do
  [ -f "$MEDIA/$f" ] || { echo "missing test media $MEDIA/$f (python3 tests/media/media.py generate)" >&2; exit 2; }
  files+=("$(native "$MEDIA/$f")")
done
ove media add "$P" "${files[@]}"
ove media list "$P"
ove media probe "$P" av1.mkv
for f in av1.mkv hevc_main10_420.mp4 prores_hq.mov; do
  ove media grab "$P" "$f" --at 0.5 --width 320 --out "$(native "$WORK/grab-$f.png")"
  [ -s "$WORK/grab-$f.png" ] || { echo "no frame decoded from $f" >&2; exit 1; }
done
ove look "$P" --count 4 --width 320 --sheet "$(native "$WORK/sheet.png")"
ove source print "$P"
ove verify "$P"
ove export "$P" "$(native "$WORK/out.mp4")" --height 360 --fps 25 2>&1 | tee "$WORK/export.log"
ove timeline export "$P" "$(native "$WORK/out.fcpxml")"

echo
echo "== outputs"
[ -s "$WORK/out.mp4" ] || { echo "export wrote no MP4" >&2; exit 1; }
if command -v ffprobe >/dev/null; then
  ffprobe -v error -show_entries stream=codec_name,width,height -show_entries format=duration -of compact "$WORK/out.mp4"
fi
# On a real Mac or PC the hardware/platform H.264 encoder should be used, not the fallback.
if grep -q "warning:" "$WORK/export.log"; then echo "note: export used a fallback encoder (see warning above)"; fi
grep -q "<fcpxml" "$WORK/out.fcpxml" || { echo "timeline export wrote no FCPXML" >&2; exit 1; }
python3 -c "import sys, xml.dom.minidom as m; m.parse(sys.argv[1])" "$(native "$WORK/out.fcpxml")"
echo "out.mp4 $(wc -c < "$WORK/out.mp4") bytes; out.fcpxml $(wc -c < "$WORK/out.fcpxml") bytes, well-formed"
echo "Smoke test passed on $(uname -s)"
