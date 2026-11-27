#!/usr/bin/env bash
# Reproducible LGPL-only FFmpeg build (shared libraries) for the app — task 1.4 of add-desktop-shell-and-native-preview.
#
#   tools/ffmpeg/build.sh [--ci] [--jobs N] [--out DIR] [--keep-src] [--target windows-x64-gnu]
#
# Output: tools/ffmpeg/out/<target>/{include,lib,bin} plus buildconf.txt, license.txt, sizes.txt and encoders.txt.
# Targets: macOS arm64 (VideoToolbox), Windows x64 (D3D11VA; run from an MSYS2 shell with MSVC on PATH), and Linux
# x64 for development (no hardware decoder). Prerequisites: a C toolchain, make, nasm, pkg-config, cmake, meson,
# ninja, dav1d (BSD-2-Clause; the only software AV1 decoder), and libvpx and opus (BSD-3; VP9 and Opus export) —
# `brew install dav1d libvpx opus nasm pkg-config cmake meson ninja`, `vcpkg install dav1d libvpx opus`, or
# `apt install libdav1d-dev libvpx-dev libopus-dev nasm cmake meson ninja-build`.
# Export encoders built from pinned, checksummed sources (fix-render-and-edit-correctness D5): OpenH264 (BSD-2-Clause,
# shared library, all targets) and, on Windows, the NVIDIA (NVENC), AMD (AMF) and Intel (QSV through the oneVPL
# dispatcher) hardware encoders. Their headers and the dispatcher are MIT; the drivers ship with the GPU.
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
# Software H.264 encoder (BSD-2-Clause), built as a shared library next to FFmpeg's.
OPENH264_VERSION="2.7.0"
OPENH264_SHA256="5178775c8b440073e70920e97d195efff721c0e56d0a78cc82c919e95d54924d"
OPENH264_URL="https://github.com/cisco/openh264/archive/refs/tags/v${OPENH264_VERSION}.tar.gz"
# Windows hardware encoders: NVENC headers (MIT; n12.1 needs NVIDIA driver 530 or later, and on an older driver the
# encoder fails to open and export tries the next one), AMF headers (MIT), the oneVPL dispatcher for QSV (MIT, static).
NVHEADERS_VERSION="12.1.14.0"
NVHEADERS_SHA256="2fefaa227d2a3b4170797796425a59d1dd2ed5fd231db9b4244468ba327acd0b"
NVHEADERS_URL="https://github.com/FFmpeg/nv-codec-headers/archive/refs/tags/n${NVHEADERS_VERSION}.tar.gz"
AMF_VERSION="1.5.3"
AMF_SHA256="0f521ad98041f8001147cded4a660127f18b1c27a801a5cd691bda1fa48381be"
AMF_URL="https://github.com/GPUOpen-LibrariesAndSDKs/AMF/releases/download/v${AMF_VERSION}/AMF-headers-v${AMF_VERSION}.tar.gz"
VPL_VERSION="2.17.0"
VPL_SHA256="4de3e2faf1e8307fb282e4a43f443191810f6a6b0a484fffa7995ba1c814c6ec"
VPL_URL="https://github.com/intel/libvpl/archive/refs/tags/v${VPL_VERSION}.tar.gz"

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

FLAGS_SHA="$(printf '%s\n' "$(sed -n '/^DEMUXERS=/,/^esac/p' "$0")" "$TARGET" "$DAV1D_SHA256" "$ZLIB_SHA256" \
  "$OPENH264_SHA256" "$NVHEADERS_SHA256" "$AMF_SHA256" "$VPL_SHA256" | (sha256sum 2>/dev/null || shasum -a 256) | cut -c1-16)"
if [ -f "$OUT/buildconf.txt" ] && grep -q "version ${FFMPEG_VERSION}" "$OUT/buildconf.txt" && grep -q "flags ${FLAGS_SHA}" "$OUT/buildconf.txt"; then
  echo "FFmpeg ${FFMPEG_VERSION} already built in $OUT"
  "$HERE/check-license.sh" "$OUT/buildconf.txt"
  exit 0
fi

# --- source -----------------------------------------------------------------------------------------------------------
fetch() { # url sha256 [name] → verified tarball in $WORK (saved as name), unpacked there
  local tarball got
  tarball="$WORK/${3:-$(basename "$1")}"
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
  for tool in "$XP-gcc" "$XP-g++" "$XP-ar" "$XP-windres" nasm meson ninja cmake make pkg-config; do
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
cpp = '$XP-g++'
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
# dav1d from vcpkg (`vcpkg install dav1d:x64-windows`; its DLL goes next to FFmpeg's). zlib is built from the same
# pinned source as the cross build and linked statically, so FFmpeg's DLLs need no zlib DLL.
if [ "$TARGET" = windows-x64 ]; then
  VCPKG_DEPS="${OVE_VCPKG_INSTALLED:-$(cygpath -u "${VCPKG_INSTALLATION_ROOT:-${VCPKG_ROOT:-C:/vcpkg}}")/installed/x64-windows}"
  for pc in dav1d vpx opus; do
    if [ ! -f "$VCPKG_DEPS/lib/pkgconfig/$pc.pc" ]; then
      echo "$pc is needed in $VCPKG_DEPS: vcpkg install dav1d libvpx opus --triplet x64-windows" >&2
      exit 2
    fi
  done
  export PKG_CONFIG_PATH="$VCPKG_DEPS/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  command -v nmake >/dev/null || { echo "nmake is needed (run inside the MSVC developer environment)" >&2; exit 2; }
  rm -rf "$DEPS" "$WORK/zlib-${ZLIB_VERSION}" && mkdir -p "$DEPS/include" "$DEPS/lib"
  fetch "$ZLIB_URL" "$ZLIB_SHA256"
  # No -Zi: a library without its .pdb makes every link warn (LNK4099).
  (cd "$WORK/zlib-${ZLIB_VERSION}" && nmake -nologo -f win32/Makefile.msc CFLAGS="-nologo -MD -W3 -O2" zlib.lib) > "$OUT/zlib.log" 2>&1 \
    || { tail -30 "$OUT/zlib.log"; exit 1; }
  cp "$WORK/zlib-${ZLIB_VERSION}"/{zlib.h,zconf.h} "$DEPS/include/"
  # zconf.h includes <unistd.h> when HAVE_UNISTD_H is defined at all; FFmpeg's config.h defines it as 0 under MSVC.
  sed -i 's|^#ifdef HAVE_UNISTD_H .*$|#if 0 /* MSVC: no unistd.h (FFmpeg defines HAVE_UNISTD_H as 0) */|' "$DEPS/include/zconf.h"
  grep -q "^#if 0 /\* MSVC: no unistd.h" "$DEPS/include/zconf.h" || { echo "zconf.h: HAVE_UNISTD_H guard not found" >&2; exit 1; }
  # configure checks zlib with -lz, which its MSVC toolchain links as zlib.lib.
  cp "$WORK/zlib-${ZLIB_VERSION}/zlib.lib" "$DEPS/lib/zlib.lib"
  # MSVC link takes -LIBPATH: (configure translates only the flags it gets from pkg-config).
  WIN_DEP_FLAGS=(--extra-cflags=-I"$(cygpath -m "$DEPS/include")"
    --extra-ldflags=-LIBPATH:"$(cygpath -m "$DEPS/lib")" --extra-ldflags=-LIBPATH:"$(cygpath -m "$VCPKG_DEPS/lib")")
fi

# --- export encoder dependencies (all targets: OpenH264; Windows: NVENC, AMF, QSV) ------------------------------------
# Installed into $ENC (headers, libraries, pkg-config files). OpenH264's shared library is copied next to FFmpeg's so
# the app finds it at run time. Nothing here is GPL or nonfree; check-license.sh checks the result.
ENC="$WORK/enc-$TARGET"
rm -rf "$ENC" && mkdir -p "$ENC/include" "$ENC/lib/pkgconfig"
export PKG_CONFIG_PATH="$ENC/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
if [ "$TARGET" = windows-x64-gnu ]; then export PKG_CONFIG_LIBDIR="$ENC/lib/pkgconfig:$PKG_CONFIG_LIBDIR"; fi
MESON_CROSS=()
if [ "$TARGET" = windows-x64-gnu ]; then MESON_CROSS=(--cross-file "$WORK/meson-$TARGET.ini"); fi

rm -rf "$WORK/openh264-${OPENH264_VERSION}" "$WORK/openh264-build"
fetch "$OPENH264_URL" "$OPENH264_SHA256" "openh264-${OPENH264_VERSION}.tar.gz"
# The encoder library only, no tests. MSVC builds use meson's Visual Studio detection (cl on PATH).
meson setup "$WORK/openh264-build" "$WORK/openh264-${OPENH264_VERSION}" ${MESON_CROSS[@]+"${MESON_CROSS[@]}"} --prefix="$ENC" \
  --libdir=lib --buildtype=release --default-library=shared -Dtests=disabled > "$OUT/openh264.log" 2>&1 \
  || { tail -40 "$OUT/openh264.log"; exit 1; }
ninja -C "$WORK/openh264-build" -j "$JOBS" install >> "$OUT/openh264.log" 2>&1 || { tail -40 "$OUT/openh264.log"; exit 1; }
rm -rf "$WORK/openh264-build"

case "$TARGET" in
  windows-x64|windows-x64-gnu)
    # NVENC: headers only; FFmpeg loads the driver's nvEncodeAPI64.dll at run time.
    rm -rf "$WORK/nv-codec-headers-n${NVHEADERS_VERSION}"
    fetch "$NVHEADERS_URL" "$NVHEADERS_SHA256" "nv-codec-headers-n${NVHEADERS_VERSION}.tar.gz"
    make -C "$WORK/nv-codec-headers-n${NVHEADERS_VERSION}" PREFIX="$ENC" install > "$OUT/nvheaders.log" 2>&1 \
      || { tail -20 "$OUT/nvheaders.log"; exit 1; }
    # AMF: headers only; FFmpeg loads the driver's amfrt64.dll at run time.
    rm -rf "$WORK/amf-headers-v${AMF_VERSION}"
    fetch "$AMF_URL" "$AMF_SHA256"
    cp -R "$WORK/amf-headers-v${AMF_VERSION}/AMF" "$ENC/include/"
    # QSV: the oneVPL dispatcher, linked statically; it finds the Intel runtime that the graphics driver installs.
    rm -rf "$WORK/libvpl-${VPL_VERSION}" "$WORK/libvpl-build"
    fetch "$VPL_URL" "$VPL_SHA256" "libvpl-${VPL_VERSION}.tar.gz"
    VPL_CMAKE=(-DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTS=OFF
      -DCMAKE_INSTALL_PREFIX="$ENC" -DCMAKE_INSTALL_LIBDIR=lib)
    if [ "$TARGET" = windows-x64-gnu ]; then
      VPL_CMAKE+=(-DCMAKE_SYSTEM_NAME=Windows -DCMAKE_C_COMPILER="$XP-gcc" -DCMAKE_CXX_COMPILER="$XP-g++"
        -DCMAKE_RC_COMPILER="$XP-windres")
    else
      # The same C runtime as FFmpeg's MSVC build (-MD).
      VPL_CMAKE+=(-G Ninja -DCMAKE_C_COMPILER=cl -DCMAKE_CXX_COMPILER=cl -DUSE_MSVC_STATIC_RUNTIME=OFF)
    fi
    cmake -S "$WORK/libvpl-${VPL_VERSION}" -B "$WORK/libvpl-build" "${VPL_CMAKE[@]}" > "$OUT/libvpl.log" 2>&1 \
      || { tail -40 "$OUT/libvpl.log"; exit 1; }
    cmake --build "$WORK/libvpl-build" --config Release -j "$JOBS" --target install >> "$OUT/libvpl.log" 2>&1 \
      || { tail -40 "$OUT/libvpl.log"; exit 1; }
    rm -rf "$WORK/libvpl-build"
    ;;
esac
# Licence texts of the encoder sources, shipped with the app's notices (tools/notices.sh copies out/<target>/licenses).
mkdir -p "$OUT/licenses"
cp "$WORK/openh264-${OPENH264_VERSION}/LICENSE" "$OUT/licenses/openh264-BSD-2-Clause.txt"
case "$TARGET" in
  windows-x64|windows-x64-gnu)
    cp "$WORK/libvpl-${VPL_VERSION}/LICENSE" "$OUT/licenses/libvpl-MIT.txt"
    # Neither header package has a separate licence file: the MIT text is the comment at the top of the headers.
    sed -n '1,/^ *\*\//p' "$WORK/nv-codec-headers-n${NVHEADERS_VERSION}/include/ffnvcodec/nvEncodeAPI.h" \
      > "$OUT/licenses/nv-codec-headers-MIT.txt"
    sed -n '/MIT license/,/THE SOFTWARE\./p' "$WORK/amf-headers-v${AMF_VERSION}/AMF/core/Version.h" \
      | sed 's|^// \{0,1\}||' > "$OUT/licenses/amf-headers-MIT.txt"
    ;;
esac

# configure compiles its checks against these headers.
if [ "$TARGET" = windows-x64 ]; then ENC_INC="$(cygpath -m "$ENC/include")"; else ENC_INC="$ENC/include"; fi
ENC_FLAGS=(--extra-cflags=-I"$ENC_INC")

# --- configuration ----------------------------------------------------------------------------------------------------
# Only what the preview and export pipeline reads (docs/spec formats for change 1): MP4/MOV/MXF/MKV/WebM containers and
# audio-only files; H.264, HEVC, ProRes, DNxHD/HR, AV1 (dav1d), VP9 video; AAC, ALAC, Opus, PCM, MP3, FLAC audio.
# Still images (imported pictures, generated images): image2 by extension, plus the PNG and JPEG pipe probes.
DEMUXERS="mov,matroska,mxf,wav,w64,aiff,ogg,mp3,flac,aac,image2,png_pipe,jpeg_pipe"
DECODERS="h264,hevc,prores,dnxhd,libdav1d,vp9,vp8,mjpeg,png"
DECODERS+=",aac,aac_fixed,alac,opus,mp3float,mp3,flac,vorbis"
DECODERS+=",pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f32be,pcm_f64le,pcm_u8"
PARSERS="h264,hevc,av1,vp9,vp8,aac,opus,flac,mpegaudio,dnxhd,prores,mjpeg,png,vorbis"
# Export (change 6, extend-export-formats-and-queue): MP4, MOV, Matroska, WebM, M4A (ipod), WAV and FLAC out; AAC,
# FLAC, PCM and Opus (libopus, BSD-3) audio; ProRes (prores_ks), DNxHR (dnxhd) and VP9 (libvpx, BSD-3) in software;
# H.264 from OpenH264 and the platform and GPU vendors' encoders, HEVC and AV1 only from the latter (below). MPEG-4
# Part 2 is the last H.264 fallback. No x264, x265, SVT-AV1 or libaom: GPL, or not yet cleared by the licence review.
EXPORT_MUXERS="mp4,mov,ipod,matroska,webm,wav,flac"
EXPORT_ENCODERS="aac,prores_ks,dnxhd,libvpx_vp9,libopus,flac,pcm_s24le,mpeg4,png,mjpeg,libopenh264"
# VP9 and Opus encoders for WebM export (BSD-3; found through pkg-config like dav1d). Required for the shipped
# targets; the Linux development and cross builds go without them when the libraries are missing (VP9, Opus and
# WebM export then report that this build cannot write them).
EXPORT_LIBS=(--enable-libvpx --enable-libopus)
if [ "$TARGET" != macos-arm64 ] && [ "$TARGET" != windows-x64 ] && ! pkg-config --exists vpx opus 2>/dev/null; then
  echo "warning: libvpx or opus not found; this build will not export VP9, Opus or WebM" >&2
  EXPORT_ENCODERS="${EXPORT_ENCODERS/,libvpx_vp9,libopus/}"
  EXPORT_LIBS=()
fi

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
  --enable-libopenh264
  ${EXPORT_LIBS[@]+"${EXPORT_LIBS[@]}"}
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
    # Vendor encoders, tried by export before Media Foundation; each fails to open on a machine without that GPU.
    # AV1 export uses only these hardware encoders (no SVT-AV1 or libaom).
    FLAGS+=(--enable-ffnvcodec --enable-nvenc --enable-encoder=h264_nvenc,hevc_nvenc,av1_nvenc)
    FLAGS+=(--enable-amf --enable-encoder=h264_amf,hevc_amf,av1_amf)
    FLAGS+=(--enable-libvpl --enable-encoder=h264_qsv,hevc_qsv,av1_qsv)
    ;;
  linux-x64) ;;
esac

cd "$SRC"
./configure "${FLAGS[@]}" "${ENC_FLAGS[@]}" > "$OUT/configure.log" 2>&1 || { tail -40 "$OUT/configure.log"; echo "--- ffbuild/config.log: the dependency checks"; grep -n -A40 -E "^(check_lib|require_pkg_config) (zlib|libdav1d)" ffbuild/config.log 2>/dev/null | head -150; echo "--- (end)"; tail -30 ffbuild/config.log 2>/dev/null; exit 1; }

# Licence gate on the configured result before spending time compiling.
grep -E '^#define (FFMPEG_CONFIGURATION|FFMPEG_LICENSE|CONFIG_GPL|CONFIG_NONFREE|CONFIG_VERSION3) ' config.h > "$OUT/license.txt"
"$HERE/check-license.sh" "$OUT/license.txt"

make -j"$JOBS" > "$OUT/make.log" 2>&1 || { tail -40 "$OUT/make.log"; exit 1; }
make install > /dev/null
if [ "$TARGET" = windows-x64 ]; then
  # FFmpeg's DLLs import dav1d's.
  for dll in dav1d vpx opus; do
    if [ -f "$VCPKG_DEPS/bin/$dll.dll" ]; then cp "$VCPKG_DEPS/bin/$dll.dll" "$OUT/bin/"; fi
  done
  # The MSVC toolchain installs the import libraries (avutil.lib, ...) next to the DLLs; the Rust bindings link
  # from lib/.
  cp "$OUT"/bin/*.lib "$OUT/lib/"
fi

# OpenH264's shared library goes next to FFmpeg's and is packaged with them.
case "$TARGET" in
  windows-x64) cp "$ENC"/bin/*openh264*.dll "$OUT/bin/" ;;
  windows-x64-gnu) cp "$ENC"/bin/*openh264*.dll "$OUT/bin/" && "$XP-strip" --strip-unneeded "$OUT"/bin/*openh264*.dll ;;
  macos-arm64) cp -P "$ENC"/lib/libopenh264*.dylib "$OUT/lib/" ;;
  linux-x64) cp -P "$ENC"/lib/libopenh264.so* "$OUT/lib/" ;;
esac

# The export encoders this target must have (fix-render-and-edit-correctness task 3.1), as the built CLI lists them.
WANT_ENCODERS="libopenh264 aac prores_ks mpeg4"
case "$TARGET" in
  macos-arm64) WANT_ENCODERS+=" h264_videotoolbox hevc_videotoolbox" ;;
  windows-x64|windows-x64-gnu) WANT_ENCODERS+=" h264_mf hevc_mf h264_nvenc hevc_nvenc h264_amf hevc_amf h264_qsv hevc_qsv" ;;
esac
if [ "$TARGET" = windows-x64-gnu ]; then
  echo "cross build: run 'bin/ffmpeg.exe -hide_banner -encoders' on Windows to list the encoders" > "$OUT/encoders.txt"
else
  PATH="$OUT/bin:$PATH" DYLD_LIBRARY_PATH="$OUT/lib" LD_LIBRARY_PATH="$OUT/lib" "$OUT/bin/ffmpeg" -hide_banner -encoders     > "$OUT/encoders.txt" 2>&1 || { cat "$OUT/encoders.txt"; exit 1; }
  for e in $WANT_ENCODERS; do
    grep -Eq "^ [A-Z.]{6} $e " "$OUT/encoders.txt" || { echo "encoder $e is missing from the build" >&2; exit 1; }
  done
  echo "encoders present: $WANT_ENCODERS"
fi

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
[ "$KEEP_SRC" = 1 ] || rm -rf "$SRC" "$WORK/dav1d-${DAV1D_VERSION}" "$WORK/zlib-${ZLIB_VERSION}" \
  "$WORK/openh264-${OPENH264_VERSION}" "$WORK/nv-codec-headers-n${NVHEADERS_VERSION}" "$WORK/amf-headers-v${AMF_VERSION}" \
  "$WORK/libvpl-${VPL_VERSION}" "$ENC"
[ "$CI" = 1 ] && echo "artefacts: $OUT/{buildconf.txt,license.txt,sizes.txt}"
exit 0
