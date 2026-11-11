#!/usr/bin/env bash
# Fail if an FFmpeg build configuration enables GPL, version-3 or nonfree components.
#
#   tools/ffmpeg/check-license.sh <file>      # config.h excerpt, buildconf.txt, or `ffmpeg -buildconf` output
#   ffmpeg -hide_banner -buildconf | tools/ffmpeg/check-license.sh -
#
# Allowed: LGPL version 2.1 or later. Exit 0 when clean, 1 with the offending markers otherwise.
set -euo pipefail

input="${1:--}"
if [ "$input" = "-" ]; then text="$(cat)"; else text="$(cat "$input")"; fi

bad=()
# configure flags (FFMPEG_CONFIGURATION / -buildconf / buildconf.txt)
for flag in --enable-gpl --enable-nonfree --enable-version3 --enable-libx264 --enable-libx265 --enable-libxvid \
            --enable-libfdk-aac --enable-postproc --enable-frei0r --enable-librubberband --enable-libvidstab \
            --enable-openssl --enable-libsmbclient; do
  if grep -q -- "$flag\b" <<<"$text"; then bad+=("$flag"); fi
done
# config.h defines
for def in CONFIG_GPL CONFIG_NONFREE CONFIG_VERSION3; do
  if grep -Eq "^#define $def 1" <<<"$text"; then bad+=("$def=1"); fi
done
# licence string
if grep -Eqi 'license: *(nonfree|.*GPL version [23] or later$|GPL)' <<<"$text" && ! grep -Eqi 'license: *LGPL version 2\.1 or later' <<<"$text"; then
  bad+=("license: $(grep -Ei 'license:' <<<"$text" | head -1)")
fi
if grep -Eq '^#define FFMPEG_LICENSE "' <<<"$text" && ! grep -Eq '^#define FFMPEG_LICENSE "LGPL version 2\.1 or later"' <<<"$text"; then
  bad+=("$(grep -E '^#define FFMPEG_LICENSE' <<<"$text")")
fi

if [ ${#bad[@]} -gt 0 ]; then
  echo "FFmpeg licence check FAILED: ${bad[*]}" >&2
  exit 1
fi
echo "FFmpeg licence check passed (LGPL 2.1+, no GPL/version3/nonfree components)"
