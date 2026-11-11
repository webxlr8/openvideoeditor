# Source this before building the Rust crates that link FFmpeg (ove-media, ove-bench, the desktop app):
#
#   source tools/ffmpeg/env.sh
#
# Uses the LGPL build from tools/ffmpeg/build.sh when it exists for this host, otherwise falls back to the system
# FFmpeg found by pkg-config (development only: distribution builds are usually GPL and must never be shipped).

_ove_ff_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) _ove_ff_target="macos-arm64" ;;
  MINGW64_NT*|MSYS_NT*) _ove_ff_target="windows-x64" ;;
  Linux-x86_64) _ove_ff_target="linux-x64" ;;
  *) _ove_ff_target="unknown" ;;
esac
_ove_ff_out="$_ove_ff_here/out/$_ove_ff_target"

if [ -f "$_ove_ff_out/buildconf.txt" ]; then
  export FFMPEG_DIR="$_ove_ff_out"
  export PKG_CONFIG_PATH="$_ove_ff_out/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  case "$_ove_ff_target" in
    macos-arm64) export DYLD_FALLBACK_LIBRARY_PATH="$_ove_ff_out/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" ;;
    windows-x64) export PATH="$_ove_ff_out/bin:$PATH" ;;
    linux-x64) export LD_LIBRARY_PATH="$_ove_ff_out/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ;;
  esac
  # Reference CLI for ove-media's decode tests (same libraries as the ones linked).
  export OVE_FFMPEG_CLI="$_ove_ff_out/bin/ffmpeg"
  echo "ove: using LGPL FFmpeg from $_ove_ff_out"
elif pkg-config --exists libavcodec libavformat libavutil libswscale libswresample 2>/dev/null; then
  unset FFMPEG_DIR
  echo "ove: using system FFmpeg $(pkg-config --modversion libavcodec) (development only — run tools/ffmpeg/build.sh for the shipping build)"
else
  echo "ove: no FFmpeg found — run tools/ffmpeg/build.sh" >&2
fi
unset _ove_ff_here _ove_ff_target _ove_ff_out
