//! `ove agents`: listing and Connect, against stand-in programs in a temporary home. No real agent is started.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A program named `name` in `dir`: a shell script on Unix, a batch file on Windows.
fn stand_in(dir: &Path, name: &str, unix: &str, windows: &str) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        std::fs::write(&p, format!("@echo off\r\n{}\r\n", windows.replace('\n', "\r\n"))).unwrap();
        p
    } else {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{unix}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
}

/// Runs `ove` with `bin` first on the search path (so a stand-in wins over anything installed) and `home` as the
/// home folder.
fn ove(bin: &Path, home: &Path, args: &[&str]) -> Output {
    let mut dirs = vec![bin.to_path_buf()];
    dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .args(args)
        .env("PATH", std::env::join_paths(dirs).unwrap())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn agents_lists_the_short_list_with_what_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, home) = (dir.path().join("bin"), dir.path().join("home"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    stand_in(&bin, "claude", "echo '9.9.9 (stand-in)'", "echo 9.9.9 (stand-in)");
    let out = ove(&bin, &home, &["agents"]);
    assert!(out.status.success(), "{}", text(&out));
    let listed = text(&out);
    assert!(listed.contains("ok   Claude: 9.9.9 (stand-in)"), "{listed}");
    for name in ["Codex", "OpenCode", "Hermes", "Kimi", "Grok"] {
        assert!(listed.contains(&format!("{name}:")), "{name} is listed: {listed}");
    }
    assert!(!listed.contains("Claude Code:"), "the agent is called Claude: {listed}");
}

#[test]
fn connect_installs_the_guide_and_registers_the_server_through_the_agents_own_command() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, home) = (dir.path().join("bin"), dir.path().join("home"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    stand_in(
        &bin,
        "claude",
        r#"here=$(dirname "$0")
if [ "$1" = "--version" ]; then echo 9.9.9; exit 0; fi
echo "$*" >> "$here/asked.log"
if [ "$1 $2" = "mcp get" ]; then [ -f "$here/added" ] && exit 0; exit 1; fi
if [ "$1 $2" = "mcp add" ]; then touch "$here/added"; echo added; exit 0; fi
exit 2"#,
        r#"if "%1"=="--version" (
  echo 9.9.9
  exit /b 0
)
echo %*>> "%~dp0asked.log"
if "%1 %2"=="mcp get" (
  if exist "%~dp0added" exit /b 0
  exit /b 1
)
if "%1 %2"=="mcp add" (
  type nul > "%~dp0added"
  echo added
  exit /b 0
)
exit /b 2"#,
    );

    let out = ove(&bin, &home, &["agents", "connect", "claude"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("Claude now has the openvideoeditor tools"), "{}", text(&out));
    let guide = home.join(".claude").join("skills").join("openvideoeditor").join("SKILL.md");
    assert!(guide.is_file(), "the editing guide is in the agent's skills folder");
    let asked = std::fs::read_to_string(bin.join("asked.log")).unwrap();
    let asked: Vec<&str> = asked.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0], "mcp get openvideoeditor");
    assert!(asked[1].starts_with("mcp add --scope user openvideoeditor -- "), "{}", asked[1]);
    assert!(asked[1].ends_with(" mcp"), "{}", asked[1]);

    let again = ove(&bin, &home, &["agents", "connect", "claude"]);
    assert!(again.status.success(), "{}", text(&again));
    assert!(text(&again).contains("Claude already had the openvideoeditor tools"), "{}", text(&again));
}

#[test]
fn connect_never_runs_an_unchecked_agent_and_says_what_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, home) = (dir.path().join("bin"), dir.path().join("home"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    // Answers `--version`; leaves a mark if it is asked anything else.
    stand_in(
        &bin,
        "grok",
        r#"if [ "$1" = "--version" ]; then echo 1.0.79; exit 0; fi
touch "$(dirname "$0")/asked-more""#,
        r#"if "%1"=="--version" (
  echo 1.0.79
  exit /b 0
)
type nul > "%~dp0asked-more""#,
    );
    let out = ove(&bin, &home, &["agents", "connect", "grok"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("In Grok, add an MCP server named openvideoeditor that runs:"), "{}", text(&out));
    assert!(!bin.join("asked-more").exists(), "the agent was asked only for its version");
}

#[test]
fn an_unknown_agent_is_refused_with_the_supported_ones() {
    let dir = tempfile::tempdir().unwrap();
    let out = ove(dir.path(), dir.path(), &["agents", "connect", "gemini"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("claude, codex, opencode, hermes, kimi, grok"), "{}", text(&out));
}
