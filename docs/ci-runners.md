# CI runners

## How each platform is built

| Platform | Built on | Notes |
| --- | --- | --- |
| Windows installer (`-setup.exe`) | The self-hosted **Linux** runners, cross-compiled with MinGW-w64 and packaged with NSIS | No Windows machine needed. The CLI is smoke-tested under Wine. A real Windows runner (optional) adds native tests |
| macOS installer (`.dmg`) | A self-hosted **Apple-silicon Mac**, or GitHub-hosted `macos-15` once Actions billing works | Apple's SDK may only be used on Apple hardware, so macOS cannot be cross-built from Linux |
| Linux (development only) | The self-hosted Linux runners | |

## Adding a runner

- **Mac** (needed for the `.dmg`): on any Apple-silicon Mac with macOS 14+, run
  `tools/runners/macos-runner.sh --repo webxlr8/openvideoeditor --token <token>`. Get the token from Settings → Actions →
  Runners → New self-hosted runner. The script installs everything below and starts the runner as a service.
- **Windows** (optional): in an elevated PowerShell on Windows 11 x64, run
  `tools\runners\windows-runner.ps1 -Repo webxlr8/openvideoeditor -Token <token>`.

## Which OSes CI runs on

The repository variable `OVE_CI_OSES` (a JSON array) picks the CI matrix. It defaults to `["linux"]`, so runs finish
even with no Mac or Windows runner. After adding a Mac runner set it to `["linux","macos"]`, and with a Windows runner
too `["linux","macos","windows"]`.

## Labels

CI, Release and Nightly run on the project's **self-hosted** GitHub Actions runners. By default a job asks for the
labels GitHub gives every self-hosted runner:

| Job OS | Default labels | Override (repository variable, JSON array) |
| --- | --- | --- |
| macOS | `["self-hosted","macOS","ARM64"]` | `OVE_RUNNER_MACOS` |
| Windows | `["self-hosted","Windows","X64"]` | `OVE_RUNNER_WINDOWS` |
| Linux | `["self-hosted","Linux","X64"]` | `OVE_RUNNER_LINUX` |

To use custom labels, set the variable in Settings → Secrets and variables → Actions → Variables, for example
`OVE_RUNNER_MACOS = ["self-hosted","mac-studio"]`. A job whose labels match no online runner waits in the queue.

## What each runner needs

All runners:
- `git`, `bash`, `python3`.
- Nothing Rust-specific: each job installs rustup (into the runner user's home) if missing, and the pinned toolchain.
- Network access to crates.io, npm and GitHub.

The workflow installs Node 22, pnpm 10 and its cached build tools itself.

macOS (Apple silicon, macOS 14+):
- Xcode command-line tools.
- Homebrew (the workflow installs `dav1d nasm pkg-config ffmpeg`).

Windows 11 x64:
- Visual Studio 2022 Build Tools with the C++ workload.
- Chocolatey, and vcpkg on `PATH` (the workflow installs `nasm`, `ffmpeg`, `dav1d`).
- MSYS2 is set up by the workflow.
- WebView2 runtime (present on Windows 11).

Linux x64 (Ubuntu 24.04), development only:
- These packages, or passwordless `sudo apt-get` so the workflow installs them: `nasm libdav1d-dev pkg-config
  ffmpeg clang libclang-dev cmake libasound2-dev libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libssl-dev
  libdbus-1-dev`.
- The release `publish` job also needs the `gh` CLI and `sha256sum`.

Runners should keep their work folder between runs (the default), so the FFmpeg build and Rust caches are reused.
