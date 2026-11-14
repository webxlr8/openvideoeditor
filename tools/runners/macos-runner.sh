#!/usr/bin/env bash
# Turns an Apple-silicon Mac into a self-hosted GitHub Actions runner for this repository, so CI tests the macOS build
# and the release workflow produces the .dmg. Run it on the Mac, as the user that should own the runner (not root):
#
#   tools/runners/macos-runner.sh --repo webxlr8/openvideoeditor --token <registration token>
#
# The registration token comes from GitHub: the repository's Settings → Actions → Runners → New self-hosted runner
# (it is shown in the "Configure" commands, valid for one hour), or `gh api -X POST
# repos/<owner>/<repo>/actions/runners/registration-token -q .token`.
#
# What it does: installs the Xcode command-line tools and Homebrew if missing, the build tools CI uses (dav1d, nasm,
# pkg-config, ffmpeg for test media, node, pnpm), downloads the official runner into ~/actions-runner, registers it
# with the labels CI asks for (self-hosted, macOS, ARM64, plus ove-macos), and installs it as a login service that
# starts with the Mac. Re-running it is safe: finished steps are skipped.
set -euo pipefail

RUNNER_VERSION="2.338.2"
REPO=""
TOKEN=""
NAME="$(scutil --get LocalHostName 2>/dev/null || hostname -s)-ove"
DIR="$HOME/actions-runner"

usage() { sed -n '2,15p' "$0"; exit "${1:-0}"; }
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) REPO="$2"; shift ;;
    --token) TOKEN="$2"; shift ;;
    --name) NAME="$2"; shift ;;
    --dir) DIR="$2"; shift ;;
    -h|--help) usage 0 ;;
    *) echo "unknown option $1" >&2; usage 2 ;;
  esac
  shift
done

[ "$(uname -s)" = "Darwin" ] || { echo "This script is for macOS." >&2; exit 1; }
[ "$(uname -m)" = "arm64" ] || { echo "The app ships for Apple silicon only; use an M1 or newer Mac." >&2; exit 1; }
[ "$(id -u)" != "0" ] || { echo "Run as a normal user, not root." >&2; exit 1; }
[ -n "$REPO" ] && [ -n "$TOKEN" ] || usage 2
major="$(sw_vers -productVersion | cut -d. -f1)"
[ "$major" -ge 14 ] || { echo "macOS 14 or later is needed (this is $(sw_vers -productVersion))." >&2; exit 1; }

step() { printf '\n== %s\n' "$1"; }

step "Xcode command-line tools"
if ! xcode-select -p >/dev/null 2>&1; then
  xcode-select --install || true
  echo "Finish the Xcode tools installer window, then run this script again."
  exit 1
fi

step "Homebrew"
if ! command -v brew >/dev/null 2>&1; then
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  eval "$(/opt/homebrew/bin/brew shellenv)"
fi
eval "$(brew shellenv)"

step "Build tools"
brew install dav1d nasm pkg-config ffmpeg node pnpm

step "GitHub Actions runner $RUNNER_VERSION in $DIR"
mkdir -p "$DIR"
cd "$DIR"
if [ ! -x ./config.sh ]; then
  pkg="actions-runner-osx-arm64-${RUNNER_VERSION}.tar.gz"
  curl -fL -o "$pkg" "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/${pkg}"
  tar xzf "$pkg"
  rm "$pkg"
fi

step "Register with github.com/$REPO"
if [ ! -f .runner ]; then
  ./config.sh --unattended --url "https://github.com/$REPO" --token "$TOKEN" --name "$NAME" \
    --labels "ove-macos" --work _work --replace
else
  echo "Already registered ($(grep -o '"agentName": *"[^"]*"' .runner || true))."
fi

step "Run as a service (starts at login)"
# The service sees Homebrew on PATH through the runner's .path file.
echo "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin" > .path
./svc.sh install >/dev/null 2>&1 || true
./svc.sh start
./svc.sh status | tail -3

cat <<EOF

Done. The runner "$NAME" should show as Idle under Settings → Actions → Runners.
It answers jobs labelled self-hosted, macOS, ARM64 (CI's default) and ove-macos.
Keep the Mac awake and logged in while builds run (System Settings → Energy: prevent automatic sleeping).
EOF
