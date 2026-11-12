//! Test-media helpers shared by the integration tests of ove-media, ove-render, ove-gpu and ove-bench. Include with
//! `#[path = "../../../tests/media/fixtures.rs"] mod fixtures;`.
//!
//! - The synthetic matrix (`tests/media/media.py generate`) is generated at most once per test process; across
//!   processes the generator's own file lock serialises runs, and its atomic renames mean no test sees a partial file.
//! - A missing tool (python3, the ffmpeg CLI, …) fails the test with a message saying what to install, unless
//!   `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` is set: then the test prints "skipped: …" and returns early. A generation
//!   failure with the tools present always fails.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Set to `1` to let tests skip (instead of fail) when a media tool is not installed.
pub const ALLOW_MISSING_ENV: &str = "OVE_ALLOW_MISSING_MEDIA_TOOLS";

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The ffmpeg CLI that *generates* fixtures (needs lavfi sources and encoders): `OVE_FIXTURE_FFMPEG`, else `ffmpeg`
/// on PATH. Not `OVE_FFMPEG_CLI`, the decode-reference CLI of the LGPL build, which has no generators.
pub fn fixture_ffmpeg() -> String {
    std::env::var("OVE_FIXTURE_FFMPEG").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "ffmpeg".into())
}

fn runs(bin: &str, arg: &str) -> bool {
    Command::new(bin).arg(arg).output().is_ok_and(|o| o.status.success())
}

/// A required tool is missing. Fails the test with `what`, or — when skipping is allowed — prints "skipped" and
/// returns false so the caller can return early.
pub fn missing(what: &str) -> bool {
    if std::env::var(ALLOW_MISSING_ENV).as_deref() == Ok("1") {
        eprintln!("skipped: {what} ({ALLOW_MISSING_ENV}=1)");
        false
    } else {
        panic!("{what}. Install it, or set {ALLOW_MISSING_ENV}=1 to skip the tests that need it.");
    }
}

/// True when `bin -version` (or `--version`) runs; otherwise [`missing`].
pub fn require_tool(bin: &str, purpose: &str) -> bool {
    runs(bin, "-version") || runs(bin, "--version") || missing(&format!("`{bin}` is not available ({purpose})"))
}

enum State {
    Ready,
    Missing(String),
    Failed(String),
}

fn generate() -> State {
    if !runs("python3", "--version") {
        return State::Missing("`python3` is not available (it runs tests/media/media.py)".into());
    }
    let ff = fixture_ffmpeg();
    if !runs(&ff, "-version") {
        return State::Missing(format!(
            "the ffmpeg CLI `{ff}` is not available (it generates tests/media; set OVE_FIXTURE_FFMPEG to another)"
        ));
    }
    let script = repo_root().join("tests/media/media.py");
    match Command::new("python3").arg(&script).arg("generate").env("OVE_FIXTURE_FFMPEG", &ff).output() {
        Ok(o) if o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr);
            for line in err.lines().filter(|l| l.starts_with("warning:")) {
                eprintln!("{line}");
            }
            State::Ready
        }
        Ok(o) => State::Failed(format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(e) => State::Failed(e.to_string()),
    }
}

/// Generates the matrix once per process. False only when a tool is missing and skipping is allowed.
pub fn media_ready() -> bool {
    static STATE: OnceLock<State> = OnceLock::new();
    match STATE.get_or_init(generate) {
        State::Ready => true,
        State::Missing(what) => missing(what),
        State::Failed(e) => panic!("generating tests/media failed:\n{e}"),
    }
}

/// Path of a matrix file. Tests call [`media_ready`] first so a missing tool can skip them.
pub fn media(name: &str) -> PathBuf {
    assert!(media_ready(), "test media are unavailable; call media_ready() at the start of the test");
    let p = repo_root().join("tests/media/files").join(name);
    assert!(p.is_file(), "{} is not part of the generated matrix", p.display());
    p
}
