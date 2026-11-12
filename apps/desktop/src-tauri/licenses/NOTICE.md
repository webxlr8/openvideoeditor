# Third-party software in OpenVideoEditor

OpenVideoEditor includes the following third-party software. Their licences are in this folder.

## FFmpeg 9.0.2: LGPL 2.1 or later

The app loads FFmpeg's shared libraries: avcodec, avformat, avutil, swscale and swresample. They are built from the
unmodified FFmpeg 9.0.2 source with only LGPL components enabled, and dynamically linked, so you can replace them with
your own build of the same version.

- Licence: `FFmpeg-LGPL-2.1.txt`.
- Source:
  - FFmpeg: https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz (sha256
    8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e).
  - The same tarball and the scripts that configure and build it (`ffmpeg-build-scripts.tar.gz`) are attached to
    every OpenVideoEditor release, next to the installers.
- Build configuration: `ffmpeg-buildconf.txt`.

FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.

## dav1d 1.5.4: BSD 2-Clause

AV1 decoding, as a shared library used by FFmpeg. Licence: `dav1d-BSD-2-Clause.txt`.

## zlib 1.3.1: zlib licence

Compression inside FFmpeg on Windows. Licence: `zlib.txt`.

## Rust crates

The app and the `ove` command line are written in Rust. The crates they include, and their licences (MIT, Apache-2.0,
BSD, ISC, Zlib, MPL-2.0 and similar permissive licences), are listed in `THIRD-PARTY-RUST.md`.

## User interface

The interface uses React, React DOM and their scheduler (MIT, © Meta Platforms, Inc. and affiliates) and the Tauri
JavaScript API (MIT or Apache-2.0, © Tauri Programme within The Commons Conservancy). Their licence files are in
`js/`.

## Speech recognition

Captions use whisper.cpp and ggml (MIT, © The ggml authors), compiled into the app. Speech models are downloaded on
first use from their original publisher and are not part of the installer.
