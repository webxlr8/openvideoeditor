//! The Agent Client Protocol client against the stand-in agent (`examples/standin_acp.rs`, built by `cargo test`):
//! a scripted turn, the harmless-agent rules at the client (layer 2), sign-in, cancel and the time limit.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ove_agents::acp::{AcpError, Allow, Event, Launch, McpServer, Session, Stop};

/// The stand-in agent: an example of this package, next to the test binaries. A full `cargo test` builds it; a
/// filtered run (`--test acp`) does not, so it is built here once when missing.
fn standin() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let exe = std::env::current_exe().unwrap();
            let dir = exe.parent().unwrap().parent().unwrap().join("examples");
            let p = dir.join(format!("standin_acp{}", std::env::consts::EXE_SUFFIX));
            if !p.is_file() {
                let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
                let ok = std::process::Command::new(cargo)
                    .args(["build", "-q", "-p", "ove-agents", "--example", "standin_acp"])
                    .status()
                    .unwrap()
                    .success();
                assert!(ok && p.is_file(), "could not build the stand-in agent ({})", p.display());
            }
            p
        })
        .clone()
}

fn launch(mode: &str) -> Launch {
    Launch { program: standin(), args: vec!["--mode".into(), mode.into()], env: vec![] }
}

fn fake_server(log: &std::path::Path) -> McpServer {
    McpServer {
        name: "openvideoeditor".into(),
        command: standin(),
        args: vec!["--mcp-fake".into(), log.display().to_string()],
    }
}

fn allow() -> Allow {
    Allow {
        server: "openvideoeditor".into(),
        tools: ["print_timeline", "apply_timeline", "look"].iter().map(|s| s.to_string()).collect(),
    }
}

const LIMIT: Duration = Duration::from_secs(20);

#[test]
fn a_scripted_turn_reads_and_edits_through_the_mcp_server() {
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("calls.jsonl");
    let mut s = Session::start(&launch("script"), &fake_server(&log), allow(), LIMIT).unwrap();
    assert_eq!(s.agent_name, "Stand-in agent");
    let mut events = Vec::new();
    let stop = s.prompt("Rename the first clip", LIMIT, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();
    assert_eq!(stop, Stop::Reason("end_turn".into()));
    let text: String =
        events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
    assert!(text.contains("Renamed the first clip."), "{text}");
    assert!(events.contains(&Event::ToolStatus { id: "t2".into(), status: "completed".into() }), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, Event::Refused { .. })), "the app's own tools are allowed: {events:?}");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.contains("\"print_timeline\""), "{calls}");
    assert!(calls.contains("Edited by stand-in"), "the edit reached the server: {calls}");
}

#[test]
fn a_hostile_agent_is_refused_everything_outside_the_apps_tools() {
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("calls.jsonl");
    let mut s = Session::start(&launch("hostile"), &fake_server(&log), allow(), LIMIT).unwrap();
    let mut events = Vec::new();
    s.prompt("Do your worst", LIMIT, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();
    let text: String =
        events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
    for what in [
        "permission:shell",
        "permission:fetch",
        "permission:write",
        "permission:disguised shell",
        "client:read",
        "client:write",
        "client:terminal",
    ] {
        assert!(text.contains(&format!("REFUSED {what}")), "{what} must be refused:\n{text}");
    }
    let refused: Vec<&String> =
        events.iter().filter_map(|e| if let Event::Refused { what } = e { Some(what) } else { None }).collect();
    assert_eq!(refused.len(), 7, "every attempt is reported: {refused:?}");
    assert!(refused.iter().any(|w| w.contains("rm -rf")), "the person sees what was tried: {refused:?}");
    assert!(refused.iter().any(|w| w.starts_with("run the command curl")), "{refused:?}");
}

#[test]
fn an_agent_that_needs_sign_in_says_so() {
    let d = tempfile::tempdir().unwrap();
    match Session::start(&launch("signin"), &fake_server(&d.path().join("l")), allow(), LIMIT) {
        Err(AcpError::SignIn { message }) => assert!(message.contains("standin login"), "{message}"),
        Err(e) => panic!("expected sign-in, got {e}"),
        Ok(_) => panic!("expected sign-in"),
    }
}

#[test]
fn stop_cancels_the_turn_and_the_time_limit_ends_it() {
    let d = tempfile::tempdir().unwrap();
    let server = fake_server(&d.path().join("l"));

    let mut s = Session::start(&launch("hang"), &server, allow(), LIMIT).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c.store(true, Ordering::SeqCst);
    });
    let stop = s.prompt("Think", LIMIT, &cancel, &mut |_| {}).unwrap();
    assert_eq!(stop, Stop::Cancelled);

    let mut s = Session::start(&launch("hang"), &server, allow(), LIMIT).unwrap();
    let t = Instant::now();
    let r = s.prompt("Think", Duration::from_secs(1), &AtomicBool::new(false), &mut |_| {});
    assert!(matches!(r, Err(AcpError::Timeout(1))), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_program_that_is_not_an_agent_fails_cleanly() {
    let d = tempfile::tempdir().unwrap();
    let not_an_agent = Launch {
        program: standin(),
        args: vec!["--mcp-fake".into(), d.path().join("x").display().to_string()],
        env: vec![],
    };
    // It answers `initialize` with an MCP result (no protocolVersion 1) and then nothing useful.
    let r = Session::start(&not_an_agent, &fake_server(&d.path().join("l")), allow(), Duration::from_secs(3));
    assert!(r.is_err());
    let missing = Launch { program: d.path().join("no-such-agent"), args: vec![], env: vec![] };
    assert!(matches!(
        Session::start(&missing, &fake_server(&d.path().join("l")), allow(), LIMIT),
        Err(AcpError::Start(_))
    ));
}
