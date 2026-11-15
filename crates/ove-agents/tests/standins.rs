//! Detection and Connect against stand-in programs that play the agents: installed, missing, hanging, printing
//! rubbish, and recording what Connect asked them. No real agent is needed, and none is ever started.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ove_agents::{ConnectStatus, Env, Limits, State, agent, connect, detect, detect_all, find_program};

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

fn env_with(dir: &Path) -> Env {
    Env { path_dirs: vec![dir.to_path_buf()], home: None, extra_dirs: vec![] }
}

fn quick() -> Limits {
    Limits { version: Duration::from_millis(1500), connect: Duration::from_secs(10) }
}

#[test]
fn an_installed_agent_is_ready_with_its_version() {
    let dir = tempfile::tempdir().unwrap();
    stand_in(dir.path(), "claude", "echo '2.1.325 (Claude Code)'", "echo 2.1.325 (Claude Code)");
    let d = detect(agent("claude").unwrap(), &env_with(dir.path()), &quick());
    assert_eq!(d.state, State::Ready);
    assert_eq!(d.version.as_deref(), Some("2.1.325 (Claude Code)"));
    assert_eq!(d.name, "Claude");
    assert!(d.connect_runs, "Claude's command is the checked one");
    assert!(d.path.unwrap().contains("claude"));
}

#[test]
fn a_missing_agent_is_not_installed_and_says_where_to_get_it() {
    let dir = tempfile::tempdir().unwrap();
    let d = detect(agent("grok").unwrap(), &env_with(dir.path()), &quick());
    assert_eq!(d.state, State::NotInstalled);
    assert_eq!((d.path, d.version), (None, None));
    assert!(d.get_it.starts_with("https://"));
    assert!(!d.connect_runs);
}

#[test]
fn an_agent_that_hangs_is_stopped_and_shown_as_found() {
    let dir = tempfile::tempdir().unwrap();
    stand_in(dir.path(), "codex", "sleep 30", ":again\ngoto again");
    let limits = Limits { version: Duration::from_millis(300), ..quick() };
    let started = Instant::now();
    let d = detect(agent("codex").unwrap(), &env_with(dir.path()), &limits);
    assert_eq!(d.state, State::Found);
    assert_eq!(d.version, None);
    assert!(started.elapsed() < Duration::from_secs(5), "the limit holds: {:?}", started.elapsed());
}

#[test]
fn rubbish_and_failures_never_become_a_version() {
    let dir = tempfile::tempdir().unwrap();
    // A program that fails: found, but its output is not taken for a version.
    stand_in(dir.path(), "kimi", "echo 'error: no config'; exit 3", "echo error: no config\nexit /b 3");
    let d = detect(agent("kimi").unwrap(), &env_with(dir.path()), &quick());
    assert_eq!((d.state, d.version), (State::Found, None));
    // A very long line is cut to 80 characters.
    let long = "v".repeat(300);
    stand_in(dir.path(), "hermes", &format!("echo {long}"), &format!("echo {long}"));
    let d = detect(agent("hermes").unwrap(), &env_with(dir.path()), &quick());
    assert_eq!(d.state, State::Ready);
    assert_eq!(d.version.map(|v| v.len()), Some(80));
    // A version printed on stderr is read too.
    stand_in(dir.path(), "opencode", "echo 1.19.5 >&2", "echo 1.19.5 1>&2");
    let d = detect(agent("opencode").unwrap(), &env_with(dir.path()), &quick());
    assert_eq!(d.version.as_deref(), Some("1.19.5"));
}

#[test]
fn every_agent_is_reported_in_order_and_the_install_folders_are_searched() {
    let path = tempfile::tempdir().unwrap();
    let extra = tempfile::tempdir().unwrap();
    stand_in(path.path(), "claude", "echo 2.0.0", "echo 2.0.0");
    stand_in(extra.path(), "grok", "echo 1.0.79", "echo 1.0.79");
    let env =
        Env { path_dirs: vec![path.path().to_path_buf()], home: None, extra_dirs: vec![extra.path().to_path_buf()] };
    let all = detect_all(&env, &quick());
    let got: Vec<(&str, State)> = all.iter().map(|d| (d.id.as_str(), d.state)).collect();
    assert_eq!(
        got,
        [
            ("claude", State::Ready),
            ("codex", State::NotInstalled),
            ("opencode", State::NotInstalled),
            ("hermes", State::NotInstalled),
            ("kimi", State::NotInstalled),
            ("grok", State::Ready),
        ]
    );
    // The search path wins over the install folders.
    stand_in(extra.path(), "claude", "echo 9.9.9", "echo 9.9.9");
    assert!(find_program(&["claude"], &env).unwrap().starts_with(path.path()));
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_run_is_not_an_agent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("claude"), "not a program").unwrap();
    assert_eq!(find_program(&["claude"], &env_with(dir.path())), None);
}

/// A stand-in Claude that keeps a log of what it was asked and remembers whether the server was added.
fn recording_claude(dir: &Path) -> PathBuf {
    stand_in(
        dir,
        "claude",
        r#"here=$(dirname "$0")
echo "$*" >> "$here/asked.log"
if [ "$1 $2" = "mcp get" ]; then [ -f "$here/added" ] && exit 0; echo "No MCP server named $3" >&2; exit 1; fi
if [ "$1 $2" = "mcp add" ]; then touch "$here/added"; echo "Added stdio MCP server openvideoeditor"; exit 0; fi
exit 2"#,
        r#"echo %*>> "%~dp0asked.log"
if "%1 %2"=="mcp get" (
  if exist "%~dp0added" exit /b 0
  echo No MCP server named %3 1>&2
  exit /b 1
)
if "%1 %2"=="mcp add" (
  type nul > "%~dp0added"
  echo Added stdio MCP server openvideoeditor
  exit /b 0
)
exit /b 2"#,
    )
}

fn asked(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("asked.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn connect_runs_the_agents_own_command_once_and_twice_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let claude = recording_claude(dir.path());
    let ove = dir.path().join("ove");
    let spec = agent("claude").unwrap();

    let first = connect(spec, &claude, &ove, &quick());
    assert_eq!(first.status, ConnectStatus::Connected, "{first:?}");
    assert!(first.detail.contains("Added stdio MCP server"));
    let log = asked(dir.path());
    assert_eq!(log.len(), 2, "asked whether it is there, then added it: {log:?}");
    assert_eq!(log[0], "mcp get openvideoeditor");
    assert!(log[1].starts_with("mcp add --scope user openvideoeditor -- "), "{}", log[1]);
    assert!(log[1].ends_with(" mcp"), "{}", log[1]);
    assert!(log[1].contains("ove"), "the path of ove is passed: {}", log[1]);

    let second = connect(spec, &claude, &ove, &quick());
    assert_eq!(second.status, ConnectStatus::AlreadyConnected);
    assert_eq!(asked(dir.path()).len(), 3, "the second time it only asks");
    assert_eq!(asked(dir.path())[2], "mcp get openvideoeditor");
}

#[test]
fn an_agent_that_refuses_is_reported_with_what_it_said() {
    let dir = tempfile::tempdir().unwrap();
    let claude =
        stand_in(dir.path(), "claude", "echo 'not signed in' >&2; exit 1", "echo not signed in 1>&2\nexit /b 1");
    let out = connect(agent("claude").unwrap(), &claude, Path::new("ove"), &quick());
    assert_eq!(out.status, ConnectStatus::Failed);
    assert!(out.detail.contains("not signed in"), "{out:?}");
}

#[test]
fn an_unchecked_or_manual_agent_is_never_run() {
    let dir = tempfile::tempdir().unwrap();
    for id in ["codex", "opencode", "hermes", "kimi", "grok"] {
        // A stand-in that would leave a mark if it were started.
        let program = stand_in(dir.path(), id, r#"touch "$(dirname "$0")/started""#, r#"type nul > "%~dp0started""#);
        let out = connect(agent(id).unwrap(), &program, Path::new("/usr/bin/ove"), &quick());
        assert_eq!(out.status, ConnectStatus::Manual, "{id}");
        assert!(out.instructions.contains("mcp"), "{id}: {}", out.instructions);
        assert!(!dir.path().join("started").exists(), "{id} was started");
    }
}
