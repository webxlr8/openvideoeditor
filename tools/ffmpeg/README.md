# FFmpeg (LGPL, shared)

The app links FFmpeg 9.0.2 built by `build.sh`: LGPL 2.1+ only, shared libraries, only the demuxers, decoders,
parsers and hardware accelerators the preview needs (VideoToolbox on macOS, D3D11VA on Windows). Software AV1 uses
dav1d (BSD-2-Clause). Export adds ProRes, DNxHR, FLAC, AAC and PCM encoders, VP9 and Opus through libvpx and
libopus (BSD-3), the MP4, MOV, Matroska, WebM, M4A, WAV and FLAC muxers, and the platform and GPU vendors' H.264,
HEVC and AV1 encoders (VideoToolbox; Media Foundation, NVENC, AMF, Quick Sync); never x264, x265 or SVT-AV1. The
build fails if GPL, version-3 or nonfree components are enabled (`check-license.sh`, run on
the configured `config.h` before compiling and on the final build configuration).

```
tools/ffmpeg/build.sh --ci          # → tools/ffmpeg/out/<target>/{include,lib,bin}, buildconf.txt, license.txt, sizes.txt
source tools/ffmpeg/env.sh          # FFMPEG_DIR, PKG_CONFIG_PATH, library path, OVE_FFMPEG_CLI for tests
cargo test -p ove-media
```

- Shipped: `libavcodec`, `libavformat`, `libavutil`, `libswscale`, `libswresample` (+ `libdav1d`). `sizes.txt` lists
  their sizes for the installer budget.
- Export encoders: `aac`, `prores_ks`, `mpeg4` and `libopenh264` everywhere; `h264/hevc/prores_videotoolbox` on
  macOS; on Windows `h264_nvenc`/`hevc_nvenc` (NVIDIA), `h264_amf`/`hevc_amf` (AMD), `h264_qsv`/`hevc_qsv` (Intel)
  and `h264_mf`/`hevc_mf` (Media Foundation, hardware MFT when present); MP4/MOV muxers. No x264/x265. The build
  checks that the CLI lists every one of them (`encoders.txt`).
- Encoder sources, pinned by version and SHA-256 in `build.sh`:
  - OpenH264 2.7.0 (BSD-2-Clause), a shared library next to FFmpeg's, packaged with them. It is
    built from source. Cisco's patent licence for H.264 covers only the binaries Cisco itself distributes; whether to
    ship our build or download Cisco's binary at first use is an open owner decision (see
    `openspec/changes/fix-render-and-edit-correctness/design.md`).
  - nv-codec-headers n12.1.14.0 (MIT): headers only; FFmpeg loads the NVIDIA driver at run time (driver 530 or later).
  - AMF headers 1.5.3 (MIT; the release archive has no licence file, the licence is in the AMF repository): headers
    only; FFmpeg loads the AMD driver at run time.
  - libvpl 2.17.0 (MIT): the oneVPL dispatcher for QSV, linked statically; it loads the Intel runtime the graphics
    driver installs.
  Each vendor encoder fails to open on a machine without that GPU or driver; export then tries the next encoder.
- Extra prerequisites for the encoders: cmake, meson and ninja on every host (the MSVC build runs meson and cmake
  with `cl` from the MSVC environment).
- Test-only: the `ffmpeg` CLI and `libavfilter`, built with a handful of muxers/encoders so decode tests can compare
  against reference output (`framemd5`, PCM) from the very same libraries. They are never packaged.
- Without a build, `env.sh` falls back to the system FFmpeg for development; distribution builds are usually GPL and
  must never be shipped.
- Windows: run from an MSYS2 shell with the MSVC environment loaded (`--toolchain=msvc`).
- Windows from Linux (`--target windows-x64-gnu`, output `out/windows-x64-gnu/`): MinGW-w64 GCC cross build with
  the same components as the native Windows build (D3D11VA, Media Foundation `h264_mf`/`hevc_mf`). dav1d 1.5.4
  (shared, `bin/libdav1d.dll`) and zlib 1.3.1 (static, inside avcodec) are built from pinned, checksummed sources;
  libgcc is linked statically, so the DLLs import only Windows system DLLs. Needs `gcc-mingw-w64-x86-64
  g++-mingw-w64-x86-64 mingw-w64-tools nasm meson ninja-build pkg-config`. Then
  `OVE_FFMPEG_TARGET=windows-x64-gnu source tools/ffmpeg/env.sh` sets up `cargo build --target
  x86_64-pc-windows-gnu` (linker, C/C++ compilers, bindgen, whisper.cpp's CMake flags, Wine as the runner) and
  `tools/package.sh --target windows-x64-gnu` builds the NSIS installer.

Not yet measured with the encoders: the rebuild needs MSYS2 with the MSVC environment on Windows and the macOS build
machine (task 3.2 records the new `sizes.txt` totals here and checks the 60 MB installer budget). Expected additions:
one OpenH264 library per platform; the vendor encoders add code to `avcodec` only (the drivers are not shipped).

Measured on the Linux dev container (x86-64, FFmpeg 9.0.2): shipped libraries total 10.3 MB (avcodec 6.8 MB, swscale
2.1 MB, avutil 1.0 MB, avformat 0.7 MB, swresample 0.1 MB), build time 1 min 47 s on 4 vCPUs.
