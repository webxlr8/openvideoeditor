#!/usr/bin/env bash
# Reproducible LGPL-only FFmpeg build (shared libraries) for the app — task 1.4 of add-desktop-shell-and-native-preview.
#
#   tools/ffmpeg/build.sh [--ci] [--jobs N] [--out DIR] [--keep-src]
#
# Output: tools/ffmpeg/out/<target>/{include,lib,bin} plus buildconf.txt, license.txt and sizes.txt.
# Targets: macOS arm64 (VideoToolbox), Windows x64 (D3D11VA; run from an MSYS2 shell with MSVC on PATH), and Linux
# x64 for development (no hardware decoder). Prerequisites: a C toolchain, make, nasm, pkg-config, and dav1d
# (BSD-2-Clause; the only software AV1 decoder) — `brew install dav1d nasm pkg-config`, `vcpkg install dav1d`, or
# `apt install libdav1d-dev nasm`.
#
# The build fails if the configuration enables GPL, version-3 or nonfree components (see check-license.sh).
set -euo pipefail

FFMPEG_VERSION="9.0.2"
FFMPEG_SHA256="8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"

HERE="$(cd "$(dirname "$0")" && pwd)"
CI=0
JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
OUT=""
KEEP_SRC=0
while [ $# -gt 0 ]; do
  case "$1" in
    --ci) CI=1 ;;
    --jobs) JOBS="$2"; shift ;;
    --out) OUT="$2"; shift ;;
    --keep-src) KEEP_SRC=1 ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) TARGET="macos-arm64" ;;
  MINGW64_NT*|MSYS_NT*) TARGET="windows-x64" ;;
  Linux-x86_64) TARGET="linux-x64" ;;
  *) echo "unsupported build host $(uname -s)-$(uname -m)" >&2; exit 2 ;;
esac
OUT="${OUT:-$HERE/out/$TARGET}"
WORK="$HERE/.work"
mkdir -p "$WORK" "$OUT"

FLAGS_SHA="$(printf '%s\n' "$(sed -n '/^DEMUXERS=/,/^esac/p' "$0")" | (sha256sum 2>/dev/null || shasum -a 256) | cut -c1-16)"
if [ -f "$OUT/buildconf.txt" ] && grep -q "version ${FFMPEG_VERSION}" "$OUT/buildconf.txt" && grep -q "flags ${FLAGS_SHA}" "$OUT/buildconf.txt"; then
  echo "FFmpeg ${FFMPEG_VERSION} already built in $OUT"
  "$HERE/check-license.sh" "$OUT/buildconf.txt"
  exit 0
fi

# --- source -----------------------------------------------------------------------------------------------------------
TARBALL="$WORK/ffmpeg-${FFMPEG_VERSION}.tar.xz"
if [ ! -f "$TARBALL" ]; then
  curl -sSfL --retry 5 --retry-all-errors -o "$TARBALL.part" "$FFMPEG_URL"
  mv "$TARBALL.part" "$TARBALL"
fi
if command -v sha256sum >/dev/null; then GOT="$(sha256sum "$TARBALL" | cut -d' ' -f1)"; else GOT="$(shasum -a 256 "$TARBALL" | cut -d' ' -f1)"; fi
if [ "$GOT" != "$FFMPEG_SHA256" ]; then
  echo "checksum mismatch for $TARBALL: $GOT" >&2
  exit 1
fi
SRC="$WORK/ffmpeg-${FFMPEG_VERSION}"
rm -rf "$SRC"
tar -xJf "$TARBALL" -C "$WORK"

# --- configuration ----------------------------------------------------------------------------------------------------
# Only what the preview and export pipeline reads (docs/spec formats for change 1): MP4/MOV/MXF/MKV/WebM containers and
# audio-only files; H.264, HEVC, ProRes, DNxHD/HR, AV1 (dav1d), VP9 video; AAC, ALAC, Opus, PCM, MP3, FLAC audio.
DEMUXERS="mov,matroska,mxf,wav,w64,aiff,ogg,mp3,flac,aac"
DECODERS="h264,hevc,prores,dnxhd,libdav1d,vp9,vp8,mjpeg,png"
DECODERS+=",aac,aac_fixed,alac,opus,mp3float,mp3,flac,vorbis"
DECODERS+=",pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f32be,pcm_f64le,pcm_u8"
PARSERS="h264,hevc,av1,vp9,vp8,aac,opus,flac,mpegaudio,dnxhd,prores,mjpeg,png,vorbis"
# Export (change 6): MP4/MOV out; AAC audio; platform H.264/HEVC encoders (below); ProRes and MPEG-4 Part 2 as
# software fallbacks that stay LGPL (no x264/x265).
EXPORT_MUXERS="mp4,mov"
EXPORT_ENCODERS="aac,prores_ks,mpeg4,png,mjpeg"

FLAGS=(
  --prefix="$OUT"
  --enable-shared --disable-static
  --disable-gpl --disable-nonfree --disable-version3
  --disable-autodetect
  --disable-programs --disable-doc --disable-debug
  --disable-avdevice --disable-network
  --disable-everything
  --enable-avcodec --enable-avformat --enable-avutil --enable-swscale --enable-swresample
  --enable-protocol=file
  --enable-demuxer="$DEMUXERS"
  --enable-decoder="$DECODERS"
  --enable-parser="$PARSERS"
  --enable-zlib
  --enable-libdav1d
  # Reference CLI for the decode tests (framemd5 / PCM output from the same libraries the app links). Packaging copies
  # only libavcodec, libavformat, libavutil, libswscale and libswresample; the CLI and libavfilter are test-only.
  --enable-ffmpeg --enable-avfilter
  --enable-filter=aresample,aformat,format,scale,null,anull,copy,acopy,setpts,asetpts,trim,atrim
  --enable-muxer=framemd5,md5,pcm_s16le,pcm_f32le,null,"$EXPORT_MUXERS"
  --enable-encoder=rawvideo,pcm_s16le,pcm_f32le,wrapped_avframe,"$EXPORT_ENCODERS"
  --enable-bsf=aac_adtstoasc,h264_mp4toannexb,hevc_mp4toannexb
  --enable-protocol=pipe
)
case "$TARGET" in
  macos-arm64)
    FLAGS+=(--enable-videotoolbox --enable-hwaccel=h264_videotoolbox,hevc_videotoolbox,prores_videotoolbox,vp9_videotoolbox,av1_videotoolbox)
    FLAGS+=(--enable-encoder=h264_videotoolbox,hevc_videotoolbox,prores_videotoolbox)
    FLAGS+=(--arch=arm64 --extra-ldflags=-Wl,-headerpad_max_install_names)
    ;;
  windows-x64)
    FLAGS+=(--toolchain=msvc --arch=x86_64 --target-os=win64)
    FLAGS+=(--enable-d3d11va --enable-hwaccel=h264_d3d11va,h264_d3d11va2,hevc_d3d11va,hevc_d3d11va2,vp9_d3d11va,vp9_d3d11va2,av1_d3d11va,av1_d3d11va2)
    # Media Foundation encoders use the GPU vendor's hardware encoder MFT when present, else Microsoft's software one.
    FLAGS+=(--enable-mediafoundation --enable-encoder=h264_mf,hevc_mf)
    ;;
  linux-x64) ;;
esac

cd "$SRC"
./configure "${FLAGS[@]}" > "$OUT/configure.log" 2>&1 || { tail -40 "$OUT/configure.log"; exit 1; }

# Licence gate on the configured result before spending time compiling.
grep -E '^#define (FFMPEG_CONFIGURATION|FFMPEG_LICENSE|CONFIG_GPL|CONFIG_NONFREE|CONFIG_VERSION3) ' config.h > "$OUT/license.txt"
"$HERE/check-license.sh" "$OUT/license.txt"

make -j"$JOBS" > "$OUT/make.log" 2>&1 || { tail -40 "$OUT/make.log"; exit 1; }
make install > /dev/null

# --- records ----------------------------------------------------------------------------------------------------------
{
  echo "ffmpeg version ${FFMPEG_VERSION} (sha256 ${FFMPEG_SHA256})"
  echo "target ${TARGET}"
  echo "flags ${FLAGS_SHA}"
  echo "configuration: $(sed -n 's/^#define FFMPEG_CONFIGURATION "\(.*\)"$/\1/p' config.h)"
  sed -n 's/^#define FFMPEG_LICENSE "\(.*\)"$/license: \1/p' config.h
} > "$OUT/buildconf.txt"
"$HERE/check-license.sh" "$OUT/buildconf.txt"

{
  total=0
  # Shipped libraries only (libavfilter and the CLI are test tools).
  for f in "$OUT"/lib/*.so.* "$OUT"/lib/*.dylib "$OUT"/bin/*.dll; do
    [ -f "$f" ] && [ ! -L "$f" ] || continue
    case "$(basename "$f")" in *avfilter*) continue ;; esac
    s=$(wc -c < "$f" | tr -d ' ')
    total=$((total + s))
    printf '%10d  %s\n' "$s" "$(basename "$f")"
  done
  printf '%10d  total (%s MB)\n' "$total" "$(awk "BEGIN { printf \"%.1f\", $total / 1048576 }")"
} > "$OUT/sizes.txt"
cat "$OUT/buildconf.txt" "$OUT/sizes.txt"

cd "$HERE"
[ "$KEEP_SRC" = 1 ] || rm -rf "$SRC"
[ "$CI" = 1 ] && echo "artefacts: $OUT/{buildconf.txt,license.txt,sizes.txt}"
exit 0
