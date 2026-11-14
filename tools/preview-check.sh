#!/usr/bin/env bash
# The picture in the app's viewer, checked on the real screen of a Mac, a Windows PC (run from Git Bash there) or a
# Linux display (with ImageMagick).
# WebDriver screenshots and frame counters cannot see a native video surface; a capture of the screen can.
#
#   tools/preview-check.sh PATH_TO_OVE PATH_TO_APP OUT.png [VIEWER]
#
# Makes a throwaway library with one project holding a solid magenta clip, starts the app on it, opens the project
# through the app's control link (`ove app open`, `ove app seek`), captures the screen to OUT.png and counts magenta
# pixels. VIEWER (`below`, `above` or `stream`) sets OVE_VIEWER_LAYERING; without it the app's default viewer is used.
# Exits 1 when the picture is not on the screen, 3 when the capture itself is blank. Needs ffmpeg (with libx264) and
# python3.
set -euo pipefail
OVE="${1:?usage: $0 PATH_TO_OVE PATH_TO_APP OUT.png [VIEWER]}"
APP="${2:?usage: $0 PATH_TO_OVE PATH_TO_APP OUT.png [VIEWER]}"
OUT="${3:?usage: $0 PATH_TO_OVE PATH_TO_APP OUT.png [VIEWER]}"
VIEWER="${4:-}"
P="Preview check"

case "$(uname -s)" in
  Darwin) OS=macos ;;
  MINGW*|MSYS*) OS=windows ;;
  Linux) OS=linux; command -v import > /dev/null || { echo "preview check: needs ImageMagick's import on Linux" >&2; exit 2; } ;;
  *) echo "preview check: no screen capture for this system" >&2; exit 2 ;;
esac
# Native path for the programs: Windows paths under Git Bash, unchanged elsewhere.
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

WORK="$(mktemp -d)"
mkdir -p "$WORK/lib" "$(dirname "$OUT")"
LIB="$(native "$WORK/lib")"
ove() { "$OVE" --library "$LIB" "$@"; }
stop_app() {
  if [ "$OS" = windows ]; then
    taskkill //F //IM "$(basename "$APP")" > /dev/null 2>&1 || true
    # taskkill returns before Windows closes the app's files; wait so the clean-up can delete them.
    for _ in $(seq 1 20); do
      tasklist //FI "IMAGENAME eq $(basename "$APP")" 2> /dev/null | grep -qi "$(basename "$APP")" || break
      sleep 0.5
    done
  elif [ -n "${pid:-}" ]; then kill "$pid" 2> /dev/null || true; fi
}
# A file that is still locked must not turn a passed check into a failure.
trap 'stop_app; rm -rf "$WORK" 2> /dev/null || true' EXIT

ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0xFF00FF:s=640x360:r=30:d=4" \
  -c:v libx264 -pix_fmt yuv420p "$WORK/magenta.mp4"
ove project create "$P" > /dev/null
ove media add "$P" "$(native "$WORK/magenta.mp4")" > /dev/null

export OVE_LIBRARY="$LIB"
if [ -n "$VIEWER" ]; then export OVE_VIEWER_LAYERING="$VIEWER"; else unset OVE_VIEWER_LAYERING; fi
"$APP" > "$WORK/app.log" 2>&1 &
pid=$!

# The control link answers once the app is up; the page needs a little longer to be ready for the project.
up=0
for _ in $(seq 1 90); do
  if ove app context > /dev/null 2>&1; then up=1; break; fi
  sleep 1
done
[ "$up" = 1 ] || { echo "preview check: the app did not answer on its control link" >&2; tail -30 "$WORK/app.log" >&2 || true; exit 1; }
sleep 15
ove app open "$P" > /dev/null
sleep 3
ove app seek 1 > /dev/null
sleep 10
ove app context | sed 's/^/  app: /'

if [ "$OS" = macos ]; then
  screencapture -x "$OUT"
elif [ "$OS" = linux ]; then
  import -window root "$OUT"
else
  powershell -NoProfile -NonInteractive -Command "
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    \$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    \$bmp = New-Object System.Drawing.Bitmap \$b.Width, \$b.Height
    [System.Drawing.Graphics]::FromImage(\$bmp).CopyFromScreen(\$b.Location, [System.Drawing.Point]::Empty, \$b.Size)
    \$bmp.Save('$(native "$OUT")')"
fi
[ -s "$OUT" ] || { echo "preview check: no screen capture was written" >&2; exit 1; }

ffmpeg -hide_banner -loglevel error -y -i "$OUT" -f rawvideo -pix_fmt rgb24 "$WORK/shot.rgb"
python3 - "$WORK/shot.rgb" "${VIEWER:-default}" <<'PY'
import sys
data = open(sys.argv[1], "rb").read()
total = len(data) // 3
r, g, b = data[0::3], data[1::3], data[2::3]
# A capture that is one flat colour is a failed capture, not a missing picture.
if total == 0 or (min(r) == max(r) and min(g) == max(g) and min(b) == max(b)):
    print("preview check: the screen capture is blank (one flat colour); nothing can be concluded from it")
    sys.exit(3)
# Magenta as a screen shows it (colour management moves the exact values): strong red and blue, little green.
magenta = sum(1 for i in range(total) if r[i] > 170 and b[i] > 170 and g[i] < 110)
share = 100.0 * magenta / max(total, 1)
print(f"preview check ({sys.argv[2]} viewer): {magenta} of {total} screen pixels are magenta ({share:.1f}%)")
# The viewer box is a tenth of the screen or more; the app's own colours and a desktop behind it have next to none.
if share < 1.5:
    sys.exit("preview check: the clip's picture is not on the screen")
PY
