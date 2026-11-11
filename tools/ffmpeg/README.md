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
- Test-only: the `ffmpeg` CLI and `libavfilter`, built with a handful of muxers/encoders so decode tests can compare
  against reference output (`framemd5`, PCM) from the very same libraries. They are never packaged.
- Without a build, `env.sh` falls back to the system FFmpeg for development; distribution builds are usually GPL and
  must never be shipped.
- Windows: run from an MSYS2 shell with the MSVC environment loaded (`--toolchain=msvc`).

Measured on the Linux dev container (x86-64, FFmpeg 9.0.2): shipped libraries total 10.3 MB (avcodec 6.8 MB, swscale
2.1 MB, avutil 1.0 MB, avformat 0.7 MB, swresample 0.1 MB), build time 1 min 47 s on 4 vCPUs.
