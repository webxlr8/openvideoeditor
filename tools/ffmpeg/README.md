# FFmpeg (LGPL, shared)

The app links FFmpeg 9.0.2 built by `build.sh`: LGPL 2.1+ only, shared libraries, only the demuxers, decoders,
parsers and hardware accelerators the preview needs (VideoToolbox on macOS, D3D11VA on Windows). Software AV1 uses
dav1d (BSD-2-Clause). The build fails if GPL, version-3 or nonfree components are enabled (`check-license.sh`, run on
the configured `config.h` before compiling and on the final build configuration).

```
tools/ffmpeg/build.sh --ci          # → tools/ffmpeg/out/<target>/{include,lib,bin}, buildconf.txt, license.txt, sizes.txt
source tools/ffmpeg/env.sh          # FFMPEG_DIR, PKG_CONFIG_PATH, library path, OVE_FFMPEG_CLI for tests
cargo test -p ove-media
```

- Shipped: `libavcodec`, `libavformat`, `libavutil`, `libswscale`, `libswresample` (+ `libdav1d`). `sizes.txt` lists
  their sizes for the installer budget.
- Export encoders (all LGPL): `aac`, `prores_ks`, `mpeg4` everywhere; `h264/hevc/prores_videotoolbox` on macOS;
  `h264_mf`/`hevc_mf` (Media Foundation, hardware MFT when present) on Windows; MP4/MOV muxers. No x264/x265.
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

Measured on the Linux dev container (x86-64, FFmpeg 9.0.2): shipped libraries total 10.3 MB (avcodec 6.8 MB, swscale
2.1 MB, avutil 1.0 MB, avformat 0.7 MB, swresample 0.1 MB), build time 1 min 47 s on 4 vCPUs.
