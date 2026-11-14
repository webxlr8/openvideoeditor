# Source this before building the Rust crates that link FFmpeg (ove-media, ove-bench, the desktop app):
#
#   source tools/ffmpeg/env.sh
#   OVE_FFMPEG_TARGET=windows-x64-gnu source tools/ffmpeg/env.sh   # cross build for Windows from Linux
#
# Uses the LGPL build from tools/ffmpeg/build.sh when it exists for this host, otherwise falls back to the system
# FFmpeg found by pkg-config (development only: distribution builds are usually GPL and must never be shipped).
#
# With OVE_FFMPEG_TARGET=windows-x64-gnu (built with `tools/ffmpeg/build.sh --target windows-x64-gnu`) it also sets up
# `cargo build --target x86_64-pc-windows-gnu`: the MinGW-w64 linker and compilers, bindgen's clang target, and a
# CMake cross configuration for whisper.cpp. There is no system fallback for a cross target.

_ove_ff_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) _ove_ff_target="macos-arm64" ;;
  MINGW64_NT*|MSYS_NT*) _ove_ff_target="windows-x64" ;;
  Linux-x86_64) _ove_ff_target="linux-x64" ;;
  *) _ove_ff_target="unknown" ;;
esac
_ove_ff_target="${OVE_FFMPEG_TARGET:-$_ove_ff_target}"
_ove_ff_out="$_ove_ff_here/out/$_ove_ff_target"

if [ "$_ove_ff_target" = windows-x64-gnu ]; then
  if [ -f "$_ove_ff_out/buildconf.txt" ]; then
    export FFMPEG_DIR="$_ove_ff_out"
    # pkg-config must never hand the host's libraries to a Windows build.
    export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_LIBDIR="$_ove_ff_out/lib/pkgconfig"
    unset PKG_CONFIG_PATH
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
    # whisper-rs-sys links the C++ runtime as `-lstdc++`; a search directory holding only libstdc++.a makes that the
    # static library, so the executables need no libstdc++-6.dll / libgcc_s_seh-1.dll next to them.
    _ove_ff_rt="$_ove_ff_out/static-runtime"
    mkdir -p "$_ove_ff_rt" && ln -sf "$(x86_64-w64-mingw32-g++ -print-file-name=libstdc++.a)" "$_ove_ff_rt/libstdc++.a"
    # GCC 13's win32 thread model puts C++ thread support (gthr-win32, used by whisper.cpp) in libgcc, which needs
    # __mingwthr_key_dtor from libmingw32; rustc's link line lists -lgcc after -lmingw32, so ask for the symbol
    # up front. RUSTFLAGS, when set (CI sets -D warnings), replaces the per-target flags, so it gets them too.
    _ove_ff_link="-L native=$_ove_ff_rt -C link-arg=-Wl,-u,__mingwthr_key_dtor"
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="$_ove_ff_link"
    if [ -n "${RUSTFLAGS:-}" ]; then
      case " $RUSTFLAGS " in *" $_ove_ff_link "*) ;; *) export RUSTFLAGS="$RUSTFLAGS $_ove_ff_link" ;; esac
    fi
    unset _ove_ff_link _ove_ff_rt
    # Run the Windows test binaries and tools (`cargo run`/`cargo test`) under Wine when it is installed.
    if command -v wine >/dev/null; then export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine; fi
    # C/C++ dependencies (cc crate, whisper.cpp through CMake, which the cmake crate configures with
    # CMAKE_SYSTEM_NAME=Windows). The same compiler as the linker, so the C++ runtime matches.
    export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
    export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar RC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-windres
    # whisper-rs-sys forwards CMAKE_* variables to whisper.cpp's CMake build: the per-configuration flags (CMake's
    # defaults plus a header that fills gaps in MinGW-w64 < 12, see tools/cross/mingw-w64-compat.h).
    _ove_ff_compat="-include $(cd "$_ove_ff_here/../cross" && pwd)/mingw-w64-compat.h"
    export CMAKE_C_FLAGS_RELEASE="-O3 -DNDEBUG $_ove_ff_compat" CMAKE_CXX_FLAGS_RELEASE="-O3 -DNDEBUG $_ove_ff_compat"
    export CMAKE_C_FLAGS_RELWITHDEBINFO="-O2 -g -DNDEBUG $_ove_ff_compat"
    export CMAKE_CXX_FLAGS_RELWITHDEBINFO="-O2 -g -DNDEBUG $_ove_ff_compat"
    unset _ove_ff_compat
    # ggml drops the `lib` prefix of its static libraries on Windows (ggml.a), but rustc's windows-gnu target only
    # looks for libggml.a. The language-specific prefix takes precedence over the one ggml sets.
    export CMAKE_STATIC_LIBRARY_PREFIX_C=lib CMAKE_STATIC_LIBRARY_PREFIX_CXX=lib
    # bindgen (FFmpeg and whisper.cpp headers) parses with the MinGW headers so `long` and friends match Windows.
    export BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_gnu="--target=x86_64-w64-mingw32 --sysroot=/usr/x86_64-w64-mingw32"
    # The FFmpeg DLLs for Windows programs run under Wine (`cargo run`, `cargo test`). Tests that need the media
    # tools (python3, ffmpeg, ffprobe from tests/media) cannot start them from inside Wine; the document, store,
    # control, interchange and CLI tests run (tools/cross/wine-smoke.sh covers the media commands).
    export WINEPATH="$_ove_ff_out/bin${WINEPATH:+;$WINEPATH}"
    echo "ove: using LGPL FFmpeg from $_ove_ff_out (cross: x86_64-pc-windows-gnu)"
  else
    echo "ove: no FFmpeg for $_ove_ff_target — run tools/ffmpeg/build.sh --target $_ove_ff_target" >&2
  fi
elif [ -z "${OVE_USE_SYSTEM_FFMPEG:-}" ] && [ -f "$_ove_ff_out/buildconf.txt" ]; then
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
