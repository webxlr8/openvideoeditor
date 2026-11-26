//! The AI agents installed on this computer (`ove-agents`): which are there, and Connect.

use std::path::PathBuf;

use serde::Serialize;
use specta::Type;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    /// Installed, and it answered with its version.
    Ready,
    /// Installed, but it did not answer in time.
    Found,
    NotInstalled,
}

/// One supported agent as found on this computer.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    /// The name to show (as the vendor allows it).
    pub name: String,
    pub vendor: String,
    pub state: AgentState,
    pub version: Option<String>,
    /// Where to get the agent.
    pub get_it: String,
    /// Whether Connect does the work (true), or the person follows `connect_instructions` (false).
    pub connect_runs: bool,
    /// The command Connect runs, or what to do by hand.
    pub connect_instructions: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AgentConnectStatus {
    Connected,
    AlreadyConnected,
    /// Nothing was run: the person follows the instructions.
    Manual,
    Failed,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnect {
    pub status: AgentConnectStatus,
    pub instructions: String,
    /// What the agent printed, or why nothing was run.
    pub detail: String,
}

/// The `ove` program that ships with the app: next to the app's own program (macOS, Windows, a development build),
/// `/usr/bin/ove` from the Linux package, else whatever `ove` the search path has.
pub(crate) fn ove_program(env: &ove_agents::Env) -> PathBuf {
    let name = if cfg!(windows) { "ove.exe" } else { "ove" };
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name)));
    beside
        .filter(|p| p.is_file())
        .or_else(|| Some(PathBuf::from("/usr/bin/ove")).filter(|p| p.is_file()))
        .or_else(|| ove_agents::find_program(&["ove"], env))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn info(d: ove_agents::Detected, ove: &std::path::Path) -> AgentInfo {
    let instructions = ove_agents::agent(&d.id).map(|s| ove_agents::connect_instructions(s, ove)).unwrap_or_default();
    AgentInfo {
        state: match d.state {
            ove_agents::State::Ready => AgentState::Ready,
            ove_agents::State::Found => AgentState::Found,
            ove_agents::State::NotInstalled => AgentState::NotInstalled,
        },
        id: d.id,
        name: d.name,
        vendor: d.vendor,
        version: d.version,
        get_it: d.get_it,
        connect_runs: d.connect_runs,
        connect_instructions: instructions,
    }
}

/// The supported agents and whether each is installed. Starts each agent's program for its version, so it runs off
/// the main thread; it reads none of an agent's files.
#[tauri::command]
#[specta::specta]
pub async fn detect_agents() -> AppResult<Vec<AgentInfo>> {
    worker::blocking("detect_agents", || {
        let env = ove_agents::Env::from_process();
        let ove = ove_program(&env);
        let found = ove_agents::detect_all(&env, &ove_agents::Limits::default());
        Ok(found
            .into_iter()
            .map(|mut d| {
                // The end-to-end suite's stand-in takes an agent's place (development builds only).
                if crate::local_agent::override_launch(&d.id).is_some() {
                    d.state = ove_agents::State::Ready;
                    d.version = Some("stand-in".into());
                }
                info(d, &ove)
            })
            .collect())
    })
    .await
}

/// Gives an installed agent the editing guide and the app's MCP server, through the agent's own command. Only a
/// command checked against a real install is run; for the others the answer says what to do.
#[tauri::command]
#[specta::specta]
pub async fn connect_agent(id: String) -> AppResult<AgentConnect> {
    worker::blocking("connect_agent", move || {
        let spec = ove_agents::agent(&id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("{id} is not an agent this app supports.")))?;
        let env = ove_agents::Env::from_process();
        let limits = ove_agents::Limits::default();
        let ove = ove_program(&env);
        let found = ove_agents::detect(spec, &env, &limits);
        let Some(program) = found.path.map(PathBuf::from) else {
            return Err(AppError::new(
                ErrorCode::NotFound,
                format!("{} is not installed on this computer.", spec.name),
            ));
        };
        // The guide first: an agent that reads skills gets it even when registering the server needs the person.
        if let Some(dir) = env.home.as_deref().and_then(|h| ove_agents::skills_dir(spec, h))
            && let Err(e) = ove_agents::install_guide(&ove, &dir, &limits)
        {
            tracing::warn!(agent = spec.id, error = %e, "the editing guide could not be installed");
        }
        let out = ove_agents::connect(spec, &program, &ove, &limits);
        Ok(AgentConnect {
            status: match out.status {
                ove_agents::ConnectStatus::Connected => AgentConnectStatus::Connected,
                ove_agents::ConnectStatus::AlreadyConnected => AgentConnectStatus::AlreadyConnected,
                ove_agents::ConnectStatus::Manual => AgentConnectStatus::Manual,
                ove_agents::ConnectStatus::Failed => AgentConnectStatus::Failed,
            },
            instructions: out.instructions,
            detail: out.detail,
        })
    })
    .await
}

/// Opens a terminal in the open project's folder (the library when none is open) with `id` (Claude or Codex)
/// running, given the editing guide and the app's MCP server for that session only (add-agent-cli-and-mcp 3.5). The
/// agent's own configuration is not changed.
#[tauri::command]
#[specta::specta]
pub async fn open_agent_terminal(app: tauri::AppHandle, id: String) -> AppResult<()> {
    use tauri::Manager;
    worker::blocking("open_agent_terminal", move || {
        let spec = ove_agents::agent(&id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("{id} is not an agent this app supports.")))?;
        let env = ove_agents::Env::from_process();
        let limits = ove_agents::Limits::default();
        let ove = ove_program(&env);
        let library = ove_store::library::resolve_library(None)?;
        let config_dir =
            app.path().app_local_data_dir().unwrap_or_else(|_| std::env::temp_dir()).join("agent-terminal");
        let start = ove_agents::terminal_start(spec, &ove, &library, &config_dir.join(format!("{}-mcp.json", spec.id)))
            .ok_or_else(|| {
                AppError::new(ErrorCode::InvalidArgument, format!("{} can't be started from the app yet.", spec.name))
            })?;
        if let Some((path, contents)) = &start.file {
            std::fs::create_dir_all(&config_dir)
                .and_then(|()| std::fs::write(path, contents))
                .map_err(|e| AppError::new(ErrorCode::Io, format!("Could not prepare {}: {e}", spec.name)))?;
        }
        let args = start.args;
        let program = ove_agents::detect(spec, &env, &limits).path.map(PathBuf::from).ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, format!("{} is not installed on this computer.", spec.name))
        })?;
        let state = app.state::<crate::AppState>();
        let dir = crate::projects::lock(&state.projects.open)?
            .as_ref()
            .map(|p| p.bundle().to_path_buf())
            .unwrap_or_else(|| library.clone());
        if let Some(skills) = env.home.as_deref().and_then(|h| ove_agents::skills_dir(spec, h))
            && let Err(e) = ove_agents::install_guide(&ove, &skills, &limits)
        {
            tracing::warn!(agent = spec.id, error = %e, "the editing guide could not be installed");
        }
        open_terminal(&dir, &program, &args, &env)
            .map_err(|e| AppError::new(ErrorCode::Io, format!("Could not open a terminal: {e}")))?;
        tracing::info!(agent = spec.id, dir = %dir.display(), "agent started in a terminal");
        Ok(())
    })
    .await
}

/// Starts `program args` in a new terminal window whose working folder is `dir`.
fn open_terminal(
    dir: &std::path::Path,
    program: &std::path::Path,
    args: &[String],
    env: &ove_agents::Env,
) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(windows)]
    {
        // Windows Terminal when it is there (`wt -d <dir> <command>`), else a console window.
        let _ = env;
        if let Some(wt) = ove_agents::find_program(&["wt"], env) {
            Command::new(wt).arg("-d").arg(dir).arg(program).args(args).spawn()?;
        } else {
            Command::new(program).args(args).current_dir(dir).spawn()?;
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let _ = env;
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        let line = std::iter::once(q(&program.display().to_string()))
            .chain(args.iter().map(|a| q(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!("cd {} && {}", q(&dir.display().to_string()), line);
        let apple = format!("tell application \"Terminal\" to do script {}", serde_json::Value::String(script));
        Command::new("osascript").args(["-e", &apple, "-e", "tell application \"Terminal\" to activate"]).spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = env;
        Command::new("x-terminal-emulator").arg("-e").arg(program).args(args).current_dir(dir).spawn()?;
        Ok(())
    }
}
