//! Finding the supported agents on this computer.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::run::{Limits, run_limited};
use crate::{AGENTS, AgentSpec, McpSetup};

/// Where programs are looked for. Built from the process by [`Env::from_process`]; tests build their own.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// The folders of the search path, in order.
    pub path_dirs: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    /// Folders installers use that a desktop app's search path often lacks (an app started from the Dock or the
    /// Start menu does not get the search path a terminal has).
    pub extra_dirs: Vec<PathBuf>,
}

impl Env {
    pub fn from_process() -> Env {
        let path_dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let mut extra_dirs = Vec::new();
        if let Some(h) = &home {
            for d in [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".opencode/bin", "bin"] {
                extra_dirs.push(d.split('/').fold(h.clone(), |p, part| p.join(part)));
            }
            if cfg!(windows) {
                extra_dirs.push(h.join("scoop").join("shims"));
            }
        }
        if cfg!(windows) {
            if let Some(d) = std::env::var_os("APPDATA") {
                extra_dirs.push(PathBuf::from(d).join("npm"));
            }
            if let Some(d) = std::env::var_os("LOCALAPPDATA") {
                extra_dirs.push(PathBuf::from(d).join("Microsoft").join("WinGet").join("Links"));
            }
        } else {
            extra_dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        }
        Env { path_dirs, home, extra_dirs }
    }

    fn search_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.path_dirs.iter().chain(&self.extra_dirs)
    }
}

#[cfg(unix)]
fn runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(p: &Path) -> bool {
    p.is_file()
}

/// The first program with one of `names` in the search path or the usual install folders.
pub fn find_program(names: &[&str], env: &Env) -> Option<PathBuf> {
    // Windows: a real program, or the `.cmd` / `.bat` file npm and other installers put on the search path.
    let endings: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat"] } else { &[""] };
    for dir in env.search_dirs() {
        for name in names {
            for ending in endings {
                let candidate = dir.join(format!("{name}{ending}"));
                if runnable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Found, and it answered with a version.
    Ready,
    /// Found, but it did not answer in time or said nothing we could read.
    Found,
    NotInstalled,
}

/// One agent as found on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub state: State,
    /// The program that was found.
    pub path: Option<String>,
    /// The first line it printed for its version, at most 80 characters.
    pub version: Option<String>,
    /// Where to get the agent.
    pub get_it: String,
    /// Whether Connect runs the agent's command for the person (a checked command), or shows what to do.
    pub connect_runs: bool,
}

/// The first line with something on it, without control characters, at most 80 characters.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let clean: String = line.chars().filter(|c| !c.is_control()).take(80).collect();
    (!clean.trim().is_empty()).then(|| clean.trim().to_string())
}

/// Looks for `spec` and asks its version. Reads no file of the agent's.
pub fn detect(spec: &AgentSpec, env: &Env, limits: &Limits) -> Detected {
    let path = find_program(spec.programs, env);
    let version = path.as_deref().and_then(|p| {
        let args: Vec<String> = spec.version_args.iter().map(|a| (*a).to_string()).collect();
        let ran = run_limited(p, &args, limits.version)?;
        ran.success.then(|| first_line(&ran.stdout).or_else(|| first_line(&ran.stderr))).flatten()
    });
    let state = match (&path, &version) {
        (Some(_), Some(_)) => State::Ready,
        (Some(_), None) => State::Found,
        (None, _) => State::NotInstalled,
    };
    Detected {
        id: spec.id.into(),
        name: spec.name.into(),
        vendor: spec.vendor.into(),
        state,
        path: path.map(|p| p.display().to_string()),
        version,
        get_it: spec.get_it.into(),
        connect_runs: matches!(spec.mcp, McpSetup::Command { checked: true, .. }),
    }
}

/// Every agent of the short list, in its order. The programs are asked at the same time, so the whole takes as long
/// as the slowest one (at most the version limit).
pub fn detect_all(env: &Env, limits: &Limits) -> Vec<Detected> {
    std::thread::scope(|s| {
        let asked: Vec<_> = AGENTS.iter().map(|spec| (spec, s.spawn(move || detect(spec, env, limits)))).collect();
        asked
            .into_iter()
            // A panic while asking one agent must not lose the others: that one is asked again here, in turn.
            .map(|(spec, handle)| handle.join().unwrap_or_else(|_| detect(spec, env, limits)))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_is_trimmed_cleaned_and_short() {
        assert_eq!(first_line("\n  2.1.0 (Claude Code)\nmore\n").as_deref(), Some("2.1.0 (Claude Code)"));
        assert_eq!(first_line("\u{1b}[1mcodex\u{7} 0.1\n").as_deref(), Some("[1mcodex 0.1"));
        assert_eq!(first_line(&"x".repeat(500)).map(|s| s.len()), Some(80));
        assert_eq!(first_line("\n \n"), None);
    }
}
