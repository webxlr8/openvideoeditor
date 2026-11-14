#!/usr/bin/env bash
# Fills apps/desktop/src-tauri/licenses/generated/ with the third-party notices that ship inside every installer:
# the Rust crates (cargo-about, tools/notices/rust.hbs, about.toml), whisper.cpp's licence, the licence files of the
# JavaScript packages the UI ships, and FFmpeg's build configuration. The committed files next to it (NOTICE.md, the
# FFmpeg / dav1d / zlib licences) explain them. Run by tools/package.sh; needs `cargo about` (cargo install
# cargo-about --features cli) and `pnpm install` done in apps/desktop.
#
#   tools/notices.sh <ffmpeg-target>      e.g. macos-arm64, windows-x64-gnu
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:?ffmpeg target, e.g. windows-x64-gnu}"
OUT="$ROOT/apps/desktop/src-tauri/licenses/generated"
rm -rf "$OUT"
mkdir -p "$OUT/js"

command -v cargo-about >/dev/null || { echo "cargo-about is needed: cargo install cargo-about --locked --features cli" >&2; exit 1; }
(cd "$ROOT" && cargo about generate --fail tools/notices/rust.hbs -o "$OUT/THIRD-PARTY-RUST.md")

whisper="$(cd "$ROOT" && cargo metadata --format-version 1 --locked 2>/dev/null \
  | python3 -c 'import json,sys; m=json.load(sys.stdin); print(next(p["manifest_path"] for p in m["packages"] if p["name"]=="whisper-rs-sys"))')"
cp "$(dirname "$whisper")/whisper.cpp/LICENSE" "$OUT/whisper.cpp-MIT.txt"

nm="$ROOT/apps/desktop/node_modules"
cp "$nm/react/LICENSE" "$OUT/js/react-MIT.txt"
cp "$nm/react-dom/LICENSE" "$OUT/js/react-dom-MIT.txt"
cp "$(find "$nm/.pnpm" -maxdepth 4 -path '*/node_modules/scheduler/LICENSE' | head -1)" "$OUT/js/scheduler-MIT.txt"
cp "$nm/@tauri-apps/api/LICENSE-MIT" "$OUT/js/tauri-api-MIT.txt"
cp "$nm/@tauri-apps/api/LICENSE-APACHE-2.0" "$OUT/js/tauri-api-Apache-2.0.txt"

cp "$ROOT/tools/ffmpeg/out/$TARGET/buildconf.txt" "$OUT/ffmpeg-buildconf.txt"
echo "notices: $(find "$OUT" -type f | wc -l) files in $OUT"
