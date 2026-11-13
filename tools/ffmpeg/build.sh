#!/usr/bin/env bash
# Reproducible LGPL-only FFmpeg build (shared libraries) for the app — task 1.4 of add-desktop-shell-and-native-preview.
#
#   tools/ffmpeg/build.sh [--ci] [--jobs N] [--out DIR] [--keep-src] [--target windows-x64-gnu]
#
# Output: tools/ffmpeg/out/<target>/{include,lib,bin} plus buildconf.txt, license.txt and sizes.txt.
# Targets: macOS arm64 (VideoToolbox), Windows x64 (D3D11VA; run from an MSYS2 shell with MSVC on PATH), and Linux
# x64 for development (no hardware decoder). Prerequisites: a C toolchain, make, nasm, pkg-config, and dav1d
# (BSD-2-Clause; the only software AV1 decoder) — `brew install dav1d nasm pkg-config`, `vcpkg install dav1d`, or
# `apt install libdav1d-dev nasm`.
# Cross build for Windows x64 on a Linux x64 host (--target windows-x64-gnu, or OVE_FFMPEG_TARGET=windows-x64-gnu):
# MinGW-w64 GCC, same components as the native Windows build; dav1d (shared) and zlib (static) are cross-built from
# pinned sources — `apt install gcc-mingw-w64-x86-64 mingw-w64-tools nasm meson ninja-build pkg-config`.
#
# The build fails if the configuration enables GPL, version-3 or nonfree components (see check-license.sh).
set -euo pipefail

FFMPEG_VERSION="9.0.2"
FFMPEG_SHA256="8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"
# Cross builds only (windows-x64-gnu); native builds use the platform's dav1d and zlib.
DAV1D_VERSION="1.5.4"
DAV1D_SHA256="686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5"
DAV1D_URL="https://downloads.videolan.org/pub/videolan/dav1d/${DAV1D_VERSION}/dav1d-${DAV1D_VERSION}.tar.xz"
ZLIB_VERSION="1.3.1"
ZLIB_SHA256="9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"
ZLIB_URL="https://github.com/madler/zlib/releases/download/v${ZLIB_VERSION}/zlib-${ZLIB_VERSION}.tar.gz"

HERE="$(cd "$(dirname "$0")" && pwd)"
CI=0
JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
OUT=""
KEEP_SRC=0
TARGET="${OVE_FFMPEG_TARGET:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --ci) CI=1 ;;
    --jobs) JOBS="$2"; shift ;;
    --out) OUT="$2"; shift ;;
    --keep-src) KEEP_SRC=1 ;;
    --target) TARGET="$2"; shift ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) HOST="macos-arm64" ;;
  MINGW64_NT*|MSYS_NT*) HOST="windows-x64" ;;
  Linux-x86_64) HOST="linux-x64" ;;
  *) echo "unsupported build host $(uname -s)-$(uname -m)" >&2; exit 2 ;;
esac
TARGET="${TARGET:-$HOST}"
case "$HOST/$TARGET" in
  "$TARGET/$TARGET" | linux-x64/windows-x64-gnu) ;;
  *) echo "cannot build target $TARGET on $HOST (cross builds: linux-x64 → windows-x64-gnu)" >&2; exit 2 ;;
esac
OUT="${OUT:-$HERE/out/$TARGET}"
WORK="$HERE/.work"
mkdir -p "$WORK" "$OUT"

FLAGS_SHA="$(printf '%s\n' "$(sed -n '/^DEMUXERS=/,/^esac/p' "$0")" "$TARGET" "$DAV1D_SHA256" "$ZLIB_SHA256" | (sha256sum 2>/dev/null || shasum -a 256) | cut -c1-16)"
if [ -f "$OUT/buildconf.txt" ] && grep -q "version ${FFMPEG_VERSION}" "$OUT/buildconf.txt" && grep -q "flags ${FLAGS_SHA}" "$OUT/buildconf.txt"; then
  echo "FFmpeg ${FFMPEG_VERSION} already built in $OUT"
  "$HERE/check-license.sh" "$OUT/buildconf.txt"
  exit 0
fi

# --- source -----------------------------------------------------------------------------------------------------------
fetch() { # url sha256 → verified tarball in $WORK, unpacked there
  local tarball got
  tarball="$WORK/$(basename "$1")"
  if [ ! -f "$tarball" ]; then
    curl -sSfL --retry 5 --retry-all-errors -o "$tarball.part" "$1"
    mv "$tarball.part" "$tarball"
  fi
  if command -v sha256sum >/dev/null; then got="$(sha256sum "$tarball" | cut -d' ' -f1)"; else got="$(shasum -a 256 "$tarball" | cut -d' ' -f1)"; fi
  if [ "$got" != "$2" ]; then
    echo "checksum mismatch for $tarball: $got" >&2
    exit 1
  fi
  tar -xf "$tarball" -C "$WORK"
}
SRC="$WORK/ffmpeg-${FFMPEG_VERSION}"
rm -rf "$SRC"
fetch "$FFMPEG_URL" "$FFMPEG_SHA256"

# --- cross dependencies (windows-x64-gnu) -----------------------------------------------------------------------------
# dav1d ships as a DLL next to FFmpeg's (BSD-2-Clause); zlib (Zlib licence) is linked statically into FFmpeg for the
# PNG codecs. Both come from pinned, checksummed releases built with the MinGW-w64 cross compiler.
DEPS="$WORK/deps-$TARGET"
if [ "$TARGET" = windows-x64-gnu ]; then
  XP=x86_64-w64-mingw32
  for tool in "$XP-gcc" "$XP-ar" "$XP-windres" nasm meson ninja pkg-config; do
    command -v "$tool" >/dev/null || { echo "missing $tool (see the prerequisites at the top of $0)" >&2; exit 2; }
  done
  rm -rf "$DEPS" && mkdir -p "$DEPS/include" "$DEPS/lib"

  rm -rf "$WORK/zlib-${ZLIB_VERSION}"
  fetch "$ZLIB_URL" "$ZLIB_SHA256"
  make -C "$WORK/zlib-${ZLIB_VERSION}" -f win32/Makefile.gcc PREFIX="$XP-" CFLAGS="-O2 -Wall" libz.a > "$OUT/zlib.log" 2>&1 \
    || { tail -20 "$OUT/zlib.log"; exit 1; }
  cp "$WORK/zlib-${ZLIB_VERSION}"/{zlib.h,zconf.h} "$DEPS/include/"
  cp "$WORK/zlib-${ZLIB_VERSION}/libz.a" "$DEPS/lib/"

  rm -rf "$WORK/dav1d-${DAV1D_VERSION}" "$WORK/dav1d-build"
  fetch "$DAV1D_URL" "$DAV1D_SHA256"
  cat > "$WORK/meson-$TARGET.ini" <<EOF
[binaries]
c = '$XP-gcc'
ar = '$XP-ar'
strip = '$XP-strip'
windres = '$XP-windres'
nasm = 'nasm'

[built-in options]
c_link_args = ['-static-libgcc']

[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF
  meson setup "$WORK/dav1d-build" "$WORK/dav1d-${DAV1D_VERSION}" --cross-file "$WORK/meson-$TARGET.ini" \
    --prefix="$OUT" --libdir=lib --buildtype=release --default-library=shared \
    -Denable_tools=false -Denable_tests=false -Denable_examples=false > "$OUT/dav1d.log" 2>&1 \
    || { tail -40 "$OUT/dav1d.log"; exit 1; }
  ninja -C "$WORK/dav1d-build" -j "$JOBS" install >> "$OUT/dav1d.log" 2>&1 || { tail -40 "$OUT/dav1d.log"; exit 1; }
  rm -rf "$WORK/dav1d-build"
  "$XP-strip" --strip-unneeded "$OUT/bin/libdav1d.dll"
  # Only the dav1d built above: never the build host's .pc files.
  export PKG_CONFIG_LIBDIR="$OUT/lib/pkgconfig"
  unset PKG_CONFIG_PATH
fi

# --- native Windows dependencies (windows-x64, MSVC) ----------------------------------------------------------------
# dav1d and zlib from vcpkg (`vcpkg install dav1d:x64-windows zlib:x64-windows`); their DLLs go next to FFmpeg's.
if [ "$TARGET" = windows-x64 ]; then
  VCPKG_DEPS="${OVE_VCPKG_INSTALLED:-$(cygpath -u "${VCPKG_INSTALLATION_ROOT:-${VCPKG_ROOT:-C:/vcpkg}}")/installed/x64-windows}"
  if [ ! -f "$VCPKG_DEPS/lib/pkgconfig/dav1d.pc" ] || [ ! -f "$VCPKG_DEPS/lib/pkgconfig/zlib.pc" ]; then
    echo "dav1d and zlib are needed in $VCPKG_DEPS: vcpkg install dav1d:x64-windows zlib:x64-windows" >&2
    exit 2
  fi
  export PKG_CONFIG_PATH="$VCPKG_DEPS/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  # configure falls back to zlib.h and -lz (z.lib under MSVC) when pkg-config does not answer; vcpkg names it zlib.lib.
  rm -rf "$DEPS" && mkdir -p "$DEPS/lib" && cp "$VCPKG_DEPS/lib/zlib.lib" "$DEPS/lib/z.lib"
  WIN_DEP_FLAGS=(--extra-cflags=-I"$(cygpath -m "$VCPKG_DEPS/include")"
    --extra-ldflags=-L"$(cygpath -m "$VCPKG_DEPS/lib")" --extra-ldflags=-L"$(cygpath -m "$DEPS/lib")")
fi

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
  windows-x64|windows-x64-gnu)
    if [ "$TARGET" = windows-x64 ]; then
      FLAGS+=(--toolchain=msvc --arch=x86_64 --target-os=win64 "${WIN_DEP_FLAGS[@]}")
    else
      # MinGW-w64 GCC cross build from Linux. libgcc is linked statically, so the DLLs import only Windows system DLLs
      # (and libdav1d.dll); zlib is a static library from $DEPS.
      FLAGS+=(--enable-cross-compile --arch=x86_64 --target-os=mingw32 --cross-prefix=x86_64-w64-mingw32-)
      FLAGS+=(--pkg-config=pkg-config --extra-cflags=-I"$DEPS/include" --extra-ldflags=-L"$DEPS/lib")
      FLAGS+=(--extra-ldflags=-static-libgcc)
    fi
    FLAGS+=(--enable-d3d11va --enable-hwaccel=h264_d3d11va,h264_d3d11va2,hevc_d3d11va,hevc_d3d11va2,vp9_d3d11va,vp9_d3d11va2,av1_d3d11va,av1_d3d11va2)
    # Media Foundation encoders use the GPU vendor's hardware encoder MFT when present, else Microsoft's software one.
    FLAGS+=(--enable-mediafoundation --enable-encoder=h264_mf,hevc_mf)
    ;;
  linux-x64) ;;
esac

cd "$SRC"
./configure "${FLAGS[@]}" > "$OUT/configure.log" 2>&1 || { tail -40 "$OUT/configure.log"; echo "--- ffbuild/config.log (end)"; tail -60 ffbuild/config.log 2>/dev/null; exit 1; }

# Licence gate on the configured result before spending time compiling.
grep -E '^#define (FFMPEG_CONFIGURATION|FFMPEG_LICENSE|CONFIG_GPL|CONFIG_NONFREE|CONFIG_VERSION3) ' config.h > "$OUT/license.txt"
"$HERE/check-license.sh" "$OUT/license.txt"

make -j"$JOBS" > "$OUT/make.log" 2>&1 || { tail -40 "$OUT/make.log"; exit 1; }
make install > /dev/null
if [ "$TARGET" = windows-x64 ]; then cp "$VCPKG_DEPS/bin/dav1d.dll" "$VCPKG_DEPS/bin/zlib1.dll" "$OUT/bin/"; fi

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
[ "$KEEP_SRC" = 1 ] || rm -rf "$SRC" "$WORK/dav1d-${DAV1D_VERSION}" "$WORK/zlib-${ZLIB_VERSION}"
[ "$CI" = 1 ] && echo "artefacts: $OUT/{buildconf.txt,license.txt,sizes.txt}"
exit 0
