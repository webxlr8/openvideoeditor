# CI runners

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
- `rustup`; the toolchain itself is installed from `rust-toolchain.toml` on first use.
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
  ffmpeg libasound2-dev libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libssl-dev
  libdbus-1-dev`.
- The release `publish` job also needs the `gh` CLI and `sha256sum`.

Runners should keep their work folder between runs (the default), so the FFmpeg build and Rust caches are reused.
