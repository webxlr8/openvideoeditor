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

# --- Editing as text, history and undo -------------------------------------------------------------------------
"$OVE" --library "$LIB" source print "$P" > "$WORK/t.tsx"
python3 - "$(native "$WORK/t.tsx")" <<'PY'
import sys
p = sys.argv[1]; s = open(p, encoding="utf-8").read()
key = '<track id="t_text" name="Text" kind="video">'
assert key in s, "text track missing from the projection"
s = s.replace(key, key + '\n      <text start={0.5} end={3} textContent="Smoke title" />', 1)
open(p, "w", encoding="utf-8").write(s)
PY
ove source check "$P" "$(native "$WORK/t.tsx")"
ove source apply "$P" "$(native "$WORK/t.tsx")"
"$OVE" --library "$LIB" source print "$P" | grep -q 'textContent="Smoke title"' || { echo "text edit not applied" >&2; exit 1; }
revs=$("$OVE" --library "$LIB" history "$P" | wc -l)
[ "$revs" -ge 3 ] || { echo "expected at least 3 revisions, got $revs" >&2; exit 1; }
ove undo "$P"
if "$OVE" --library "$LIB" source print "$P" | grep -q 'Smoke title'; then echo "undo did not remove the text" >&2; exit 1; fi
ove verify "$P"

# --- Audio analysis and frame statistics ---------------------------------------------------------------------
ove audio levels "$P" audio_aac.m4a
ove audio silence "$P" audio_aac.m4a
ove audio loudness "$P" audio_aac.m4a
ove look "$P" --count 3 --stats

# --- A second export with other settings ---------------------------------------------------------------------
ove export "$P" "$(native "$WORK/out720.mp4")" --height 720 --fps 24
[ -s "$WORK/out720.mp4" ] || { echo "720p export wrote nothing" >&2; exit 1; }

# --- Timeline exchange round trip: every format back into a new project --------------------------------------
for ext in fcpxml xml edl otio; do
  ove timeline export "$P" "$(native "$WORK/rt.$ext")"
  ove project create "RT $ext"
  ove timeline import "RT $ext" "$(native "$WORK/rt.$ext")"
  n=$("$OVE" --library "$LIB" source print "RT $ext" | grep -c '<clip' || true)
  [ "$n" -ge 1 ] || { echo "round trip through .$ext imported no clips" >&2; exit 1; }
  echo "round trip .$ext: $n clips"
done

# --- MCP server: the protocol an AI agent uses ---------------------------------------------------------------
step mcp
python3 - "$OVE" "$LIB" "$P" <<'PY'
import json, subprocess, sys
ove, lib, project = sys.argv[1:4]
proc = subprocess.Popen([ove, "--library", lib, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def call(i, method, params=None):
    msg = {"jsonrpc": "2.0", "id": i, "method": method}
    if params is not None:
        msg["params"] = params
    proc.stdin.write(json.dumps(msg) + "\n"); proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        if not line:
            sys.exit(f"MCP server closed during {method}")
        reply = json.loads(line)
        if reply.get("id") == i:
            if "error" in reply:
                sys.exit(f"MCP {method} failed: {reply['error']}")
            return reply["result"]
init = call(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke", "version": "1"}})
proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n"); proc.stdin.flush()
tools = [t["name"] for t in call(2, "tools/list")["tools"]]
for needed in ("list_projects", "print_timeline", "apply_timeline", "verify_timeline"):
    assert needed in tools, f"MCP tool {needed} missing"
listed = call(3, "tools/call", {"name": "list_projects", "arguments": {}})
assert not listed.get("isError"), listed
printed = call(4, "tools/call", {"name": "print_timeline", "arguments": {"project": project}})
text = "".join(c.get("text", "") for c in printed.get("content", []))
assert "<timeline" in text, "print_timeline returned no timeline"
proc.stdin.close(); proc.wait(timeout=30)
print(f"MCP ok: server {init.get('serverInfo', {}).get('name')}, {len(tools)} tools")
PY

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
