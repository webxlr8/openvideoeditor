//! The AI agents a person may already have installed, and how the app works with them.
//!
//! - [`AGENTS`]: the short list. Each agent is one data entry; nothing else in the app names an agent.
//! - [`detect`]: finds an agent's program and reads its version, without touching its credentials.
//! - [`connect`]: gives an agent used from its own terminal the app's editing guide and MCP server, by running the
//!   agent's own command. Only a command checked against a real install is run; for the others the app shows what
//!   to do ([`connect_instructions`]).
//!
//! The app never shows an agent's sign-in and never reads, stores or forwards its credentials: it starts the program
//! the person installed, as that person.
#![forbid(unsafe_code)]

pub mod acp;
mod detect;
mod run;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use detect::{Detected, Env, State, detect, detect_all, find_program};
pub use run::Limits;

/// The name the app's MCP server is registered under in every agent.
pub const SERVER_NAME: &str = "openvideoeditor";

/// The tools of the MCP server the in-app chat gives an agent (`ove mcp --project`): seeing and editing the open
/// project, nothing else. The server offers only these, and the chat grants an agent's permission requests only for
/// these (design D6 layers 2 and 3).
pub const APP_TOOLS: &[&str] = &[
    "show_project",
    "print_timeline",
    "check_timeline",
    "apply_timeline",
    "verify_timeline",
    "timeline_history",
    "undo_timeline",
    "list_media",
    "probe_media",
    "grab_frame",
    "filmstrip",
    "detect_scenes",
    "audio_levels",
    "waveform_image",
    "detect_silence",
    "measure_loudness",
    "look",
    "app_context",
    "seek",
    "play",
    "pause",
    "docs",
];

/// How an agent is told about an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSetup {
    /// The agent's own command. `add` is run with `{ove}` replaced by the path of the `ove` program; `registered`,
    /// when given, exits with 0 if the server is already there. `checked` is true only when the command was run by
    /// us against a real install of the agent: an unchecked command is shown to copy, never run.
    Command { add: &'static [&'static str], registered: Option<&'static [&'static str]>, checked: bool },
    /// No command we know: the person adds the server in the agent's settings, following `docs`.
    Manual { docs: &'static str },
}

/// How an agent is started as an Agent Client Protocol agent (JSON-RPC on stdin and stdout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpStart {
    /// The agent's own program with these arguments.
    Own(&'static [&'static str]),
    /// A separate adapter program (an npm package, at a pinned version) that drives the installed agent; started
    /// with `npx`.
    Adapter { package: &'static str, version: &'static str },
}

/// One supported agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentSpec {
    /// Stable id used in settings and on the command line.
    pub id: &'static str,
    /// The name shown to people, as the vendor allows it (Anthropic: "Claude", not "Claude Code").
    pub name: &'static str,
    pub vendor: &'static str,
    /// Program names to look for, without an extension.
    pub programs: &'static [&'static str],
    pub version_args: &'static [&'static str],
    pub acp: AcpStart,
    pub mcp: McpSetup,
    /// The agent's skills folder under the home folder, when it reads skills in the `SKILL.md` layout.
    pub skills_dir: Option<&'static str>,
    /// Where to get the agent.
    pub get_it: &'static str,
}

/// The short list, in the order it is shown. Chosen for use and for a documented way to be driven by another
/// program; see openspec/changes/add-local-agents-and-onboarding/design.md (D1).
pub const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        id: "claude",
        name: "Claude",
        vendor: "Anthropic",
        programs: &["claude"],
        version_args: &["--version"],
        // Checked on npm 2026-11-26 (the package Zed's adapter is published as).
        acp: AcpStart::Adapter { package: "@agentclientprotocol/claude-agent-acp", version: "0.112.0" },
        // Checked against Claude Code 2.1.325: `claude mcp add [options] <name> <commandOrUrl> [args...]`, default
        // scope `local` (one folder), so `--scope user` makes the server available wherever the agent is started.
        mcp: McpSetup::Command {
            add: &["mcp", "add", "--scope", "user", SERVER_NAME, "--", "{ove}", "mcp"],
            registered: Some(&["mcp", "get", SERVER_NAME]),
            checked: true,
        },
        skills_dir: Some(".claude/skills"),
        get_it: "https://claude.com/product/claude-code",
    },
    AgentSpec {
        id: "codex",
        name: "Codex",
        vendor: "OpenAI",
        programs: &["codex"],
        version_args: &["--version"],
        // Checked on npm 2026-11-26; `@zed-industries/codex-acp` is the old name (0.16).
        acp: AcpStart::Adapter { package: "@agentclientprotocol/codex-acp", version: "2.12.1" },
        // Checked against codex-cli 0.101.0 (2026-11-26): `codex mcp add <NAME> -- <COMMAND>...` adds a global server
        // without asking anything; `codex mcp get <NAME>` exits 0 when it is there and 1 when it is not.
        mcp: McpSetup::Command {
            add: &["mcp", "add", SERVER_NAME, "--", "{ove}", "mcp"],
            registered: Some(&["mcp", "get", SERVER_NAME]),
            checked: true,
        },
        skills_dir: Some(".codex/skills"),
        get_it: "https://developers.openai.com/codex/cli",
    },
    AgentSpec {
        id: "opencode",
        name: "OpenCode",
        vendor: "Anomaly",
        programs: &["opencode"],
        version_args: &["--version"],
        acp: AcpStart::Own(&["acp"]),
        // OpenCode 1.18.15's `opencode mcp add` asks for a local server's command interactively: set up by hand.
        mcp: McpSetup::Manual { docs: "https://opencode.ai/docs/mcp-servers/" },
        skills_dir: None,
        get_it: "https://opencode.ai",
    },
    AgentSpec {
        id: "hermes",
        name: "Hermes",
        vendor: "Nous Research",
        programs: &["hermes"],
        version_args: &["--version"],
        acp: AcpStart::Own(&["acp"]),
        // Checked against Hermes 0.21.5 (2026-11-26): the command connects to the server and then asks which of its
        // tools to enable, so it is shown to copy and run in a terminal, never run by the app.
        mcp: McpSetup::Command {
            add: &["mcp", "add", SERVER_NAME, "--command", "{ove}", "--args", "mcp"],
            registered: None,
            checked: false,
        },
        skills_dir: None,
        get_it: "https://hermes-agent.nousresearch.com",
    },
    AgentSpec {
        id: "kimi",
        name: "Kimi",
        vendor: "Moonshot AI",
        programs: &["kimi"],
        version_args: &["--version"],
        acp: AcpStart::Own(&["acp"]),
        mcp: McpSetup::Manual { docs: "https://github.com/MoonshotAI/kimi-code" },
        skills_dir: None,
        get_it: "https://github.com/MoonshotAI/kimi-code",
    },
    AgentSpec {
        id: "grok",
        name: "Grok",
        vendor: "xAI",
        programs: &["grok"],
        version_args: &["--version"],
        acp: AcpStart::Own(&["agent", "stdio"]),
        mcp: McpSetup::Manual { docs: "https://docs.x.ai/build/overview" },
        skills_dir: None,
        get_it: "https://docs.x.ai/build/overview",
    },
];

/// The entry with this id.
pub fn agent(id: &str) -> Option<&'static AgentSpec> {
    AGENTS.iter().find(|a| a.id == id)
}

/// How to start `spec` as an Agent Client Protocol agent: its own program (found by [`detect`]) with its arguments,
/// or `npx` running its adapter at the pinned version. `None` when the program (or `npx`) is not there.
pub fn launch(spec: &AgentSpec, program: Option<&Path>, env: &Env) -> Option<acp::Launch> {
    match spec.acp {
        AcpStart::Own(args) => Some(acp::Launch {
            program: program?.to_path_buf(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: Vec::new(),
        }),
        AcpStart::Adapter { package, version } => {
            // The adapter drives the installed agent, so the agent itself must be there too.
            program?;
            Some(acp::Launch {
                program: find_program(&["npx"], env)?,
                args: vec!["-y".into(), format!("{package}@{version}")],
                env: Vec::new(),
            })
        }
    }
}

/// What Connect did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectStatus {
    /// The agent's command ran and registered the server.
    Connected,
    /// The server was already registered; nothing was changed.
    AlreadyConnected,
    /// Nothing was run: the person follows the instructions.
    Manual,
    /// The agent's command failed; `detail` holds what it printed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectOutcome {
    pub status: ConnectStatus,
    /// What was run, or what to do by hand.
    pub instructions: String,
    /// What the agent printed (trimmed), or why nothing was run.
    pub detail: String,
}

/// One argument as a person would type it in a terminal.
fn shown(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@\\".contains(c)) {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('"', "\\\""))
    }
}

fn filled(args: &[&str], ove: &Path) -> Vec<String> {
    args.iter().map(|a| if *a == "{ove}" { ove.display().to_string() } else { (*a).to_string() }).collect()
}

/// What a person does to give `spec` the app's MCP server: the agent's command when there is one, else a sentence.
pub fn connect_instructions(spec: &AgentSpec, ove: &Path) -> String {
    match spec.mcp {
        McpSetup::Command { add, .. } => {
            let program = spec.programs.first().copied().unwrap_or(spec.id);
            std::iter::once(program.to_string())
                .chain(filled(add, ove))
                .map(|a| shown(&a))
                .collect::<Vec<_>>()
                .join(" ")
        }
        McpSetup::Manual { docs } => format!(
            "In {}, add an MCP server named {SERVER_NAME} that runs: {} mcp   ({docs})",
            spec.name,
            shown(&ove.display().to_string())
        ),
    }
}

/// The folder `spec` reads skills from, under `home`.
pub fn skills_dir(spec: &AgentSpec, home: &Path) -> Option<PathBuf> {
    spec.skills_dir.map(|d| d.split('/').fold(home.to_path_buf(), |p, part| p.join(part)))
}

/// Writes the app's editing guide into `dir` (an agent's skills folder) by asking `ove skills install`. The app uses
/// this; the command line writes the guide itself.
pub fn install_guide(ove: &Path, dir: &Path, limits: &Limits) -> Result<(), String> {
    let args = ["skills".to_string(), "install".to_string(), "--dir".to_string(), dir.display().to_string()];
    match run::run_limited(ove, &args, limits.connect) {
        Some(r) if r.success => Ok(()),
        Some(r) => Err(r.text()),
        None => Err(format!("{} could not be started, or did not answer in time", ove.display())),
    }
}

/// Gives the installed agent at `program` the app's MCP server (`ove mcp`, with `ove` at this path).
///
/// Runs the agent's own command, and only when it is a checked one; otherwise returns what to do by hand. Asking
/// twice changes nothing the second time. The app's editing guide is installed by the caller (`ove skills install`).
pub fn connect(spec: &AgentSpec, program: &Path, ove: &Path, limits: &Limits) -> ConnectOutcome {
    let instructions = connect_instructions(spec, ove);
    let McpSetup::Command { add, registered, checked: true } = spec.mcp else {
        let why = match spec.mcp {
            McpSetup::Command { .. } => {
                "this command has not been checked against a real install yet, so it is not run for you"
            }
            McpSetup::Manual { .. } => "this agent is set up in its own settings",
        };
        return ConnectOutcome { status: ConnectStatus::Manual, instructions, detail: why.into() };
    };
    if let Some(args) = registered {
        let asked = run::run_limited(program, &filled(args, ove), limits.connect);
        if asked.as_ref().is_some_and(|r| r.success) {
            return ConnectOutcome {
                status: ConnectStatus::AlreadyConnected,
                instructions,
                detail: asked.map(|r| r.text()).unwrap_or_default(),
            };
        }
    }
    match run::run_limited(program, &filled(add, ove), limits.connect) {
        Some(r) if r.success => {
            tracing::info!(agent = spec.id, "MCP server registered with the agent");
            ConnectOutcome { status: ConnectStatus::Connected, instructions, detail: r.text() }
        }
        Some(r) if r.text().to_lowercase().contains("already exists") => {
            ConnectOutcome { status: ConnectStatus::AlreadyConnected, instructions, detail: r.text() }
        }
        Some(r) => {
            tracing::warn!(agent = spec.id, "the agent refused the MCP server");
            ConnectOutcome { status: ConnectStatus::Failed, instructions, detail: r.text() }
        }
        None => ConnectOutcome {
            status: ConnectStatus::Failed,
            instructions,
            detail: format!("{} did not answer in time, or could not be started", spec.name),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_short_list_is_six_agents_with_distinct_ids_and_programs() {
        let ids: Vec<&str> = AGENTS.iter().map(|a| a.id).collect();
        assert_eq!(ids, ["claude", "codex", "opencode", "hermes", "kimi", "grok"]);
        let mut programs: Vec<&str> = AGENTS.iter().flat_map(|a| a.programs.iter().copied()).collect();
        programs.sort_unstable();
        programs.dedup();
        assert_eq!(programs.len(), 6);
    }

    #[test]
    fn claude_is_never_called_claude_code() {
        let claude = agent("claude").unwrap();
        assert_eq!(claude.name, "Claude");
        assert!(AGENTS.iter().all(|a| !a.name.contains("Claude Code")));
    }

    #[test]
    fn only_a_checked_command_is_ever_run() {
        for a in AGENTS {
            if let McpSetup::Command { checked, .. } = a.mcp {
                // Run against a real install: Claude (2.1.325) and Codex (0.101.0). Hermes's asks questions.
                assert_eq!(
                    checked,
                    a.id == "claude" || a.id == "codex",
                    "{}: mark a command checked only after running it for real",
                    a.id
                );
            }
        }
    }

    #[test]
    fn instructions_show_the_command_or_a_sentence() {
        let ove = Path::new("/Applications/Open Video Editor.app/Contents/MacOS/ove");
        assert_eq!(
            connect_instructions(agent("claude").unwrap(), ove),
            "claude mcp add --scope user openvideoeditor -- \"/Applications/Open Video Editor.app/Contents/MacOS/ove\" mcp"
        );
        assert_eq!(
            connect_instructions(agent("codex").unwrap(), Path::new("/usr/bin/ove")),
            "codex mcp add openvideoeditor -- /usr/bin/ove mcp"
        );
        assert_eq!(
            connect_instructions(agent("hermes").unwrap(), Path::new("/usr/bin/ove")),
            "hermes mcp add openvideoeditor --command /usr/bin/ove --args mcp"
        );
        let grok = connect_instructions(agent("grok").unwrap(), Path::new("/usr/bin/ove"));
        assert!(grok.starts_with("In Grok, add an MCP server named openvideoeditor that runs: /usr/bin/ove mcp"));
    }

    #[test]
    fn skills_folders() {
        let home = Path::new("/home/a");
        assert_eq!(skills_dir(agent("claude").unwrap(), home), Some(home.join(".claude").join("skills")));
        assert_eq!(skills_dir(agent("grok").unwrap(), home), None);
    }
}
