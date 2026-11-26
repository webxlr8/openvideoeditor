//! Starts an installed agent through the app's Agent Client Protocol client and sends one short prompt: a check
//! against a real install (add-local-agents-and-onboarding 3.7).
//!
//! ```text
//! cargo run -p ove-agents --example acp_probe -- <agent id> [prompt]
//! ```
//! The session gets a stand-in MCP server that only records calls (`standin_acp --mcp-fake`), so nothing is edited.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr, reason = "manual tool")]

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use ove_agents::acp::{Allow, Event, McpServer, Session};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let id = args.get(1).expect("usage: acp_probe <agent id> [prompt]");
    let prompt = args.get(2).cloned().unwrap_or_else(|| "Reply with only the word OK.".into());
    let spec = ove_agents::agent(id).expect("unknown agent id");
    let env = ove_agents::Env::from_process();
    let detected = ove_agents::detect(spec, &env, &ove_agents::Limits::default());
    println!("{}: {:?} {}", spec.name, detected.state, detected.version.clone().unwrap_or_default());
    let Some(launch) = ove_agents::launch(spec, detected.path.as_deref().map(std::path::Path::new), &env) else {
        println!("not installed (or npx missing)");
        return;
    };
    println!("start: {} {}", launch.program.display(), launch.args.join(" "));
    let exe = std::env::current_exe().unwrap();
    let standin = exe.parent().unwrap().join(format!("standin_acp{}", std::env::consts::EXE_SUFFIX));
    let log = std::env::temp_dir().join(format!("acp-probe-{id}.jsonl"));
    // `OVE_PROBE_PROJECT=<bundle>` (and `OVE_PROBE_LIBRARY`): the real pinned server, `ove mcp --project`, next to
    // this program; the agent's edits are written to that project.
    let server = match std::env::var("OVE_PROBE_PROJECT") {
        Ok(bundle) => McpServer {
            name: ove_agents::SERVER_NAME.into(),
            command: exe.parent().unwrap().parent().unwrap().join(format!("ove{}", std::env::consts::EXE_SUFFIX)),
            args: vec![
                "--library".into(),
                std::env::var("OVE_PROBE_LIBRARY").expect("OVE_PROBE_LIBRARY"),
                "mcp".into(),
                "--project".into(),
                bundle,
            ],
        },
        Err(_) => McpServer {
            name: ove_agents::SERVER_NAME.into(),
            command: standin,
            args: vec!["--mcp-fake".into(), log.display().to_string()],
        },
    };
    let allow = Allow {
        server: ove_agents::SERVER_NAME.into(),
        tools: ove_agents::APP_TOOLS.iter().map(|s| s.to_string()).collect(),
    };
    let t = Instant::now();
    let mut s = match Session::start(&launch, &server, allow, Duration::from_secs(180)) {
        Ok(s) => s,
        Err(e) => {
            println!("start failed after {:.1} s: {e}", t.elapsed().as_secs_f64());
            return;
        }
    };
    println!("session {} with {:?} after {:.1} s", s.session_id, s.agent_name, t.elapsed().as_secs_f64());
    let t = Instant::now();
    let mut text = String::new();
    let r = s.prompt(&prompt, Duration::from_secs(180), &AtomicBool::new(false), &mut |e| match e {
        Event::Text(x) => text.push_str(&x),
        other => println!("  event: {other:?}"),
    });
    println!("answer: {:?}", text.trim());
    println!("stop: {r:?} after {:.1} s", t.elapsed().as_secs_f64());
}
