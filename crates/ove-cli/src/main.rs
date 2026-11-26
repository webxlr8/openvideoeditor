//! `ove`: the command line for OpenVideoEditor projects. Works on project bundles with the app closed; when the app
//! has a project open, edits go through it (control link) so the editor updates at once. `ove mcp` serves the same
//! commands as MCP tools. Results go to stdout (one JSON object with `--json`), errors to stderr. Exit codes: 0 ok,
//! 1 refused, 2 usage, 3 the app is not running (app commands).
#![forbid(unsafe_code)]
// Without media support some options and helpers are parsed but unused.
#![cfg_attr(not(feature = "media"), allow(dead_code))]

mod analyze;
mod app;
mod common;
mod docs;
mod generate;
mod mcp;
mod media;
mod project;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use ove_core::revision::Source;
use ove_store::project::PROJECTION_FILE;
use serde_json::json;

use common::{Ctx, Fail, Out};

#[derive(Parser)]
#[command(name = "ove", version, about = "Edit projects from the command line")]
struct Cli {
    /// Library folder (default: OVE_LIBRARY, then the platform default).
    #[arg(long, global = true)]
    library: Option<PathBuf>,
    /// Print one JSON object on stdout.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Projects in the library.
    Project {
        #[command(subcommand)]
        cmd: project::ProjectCmd,
    },
    /// The files a project uses.
    Media {
        #[command(subcommand)]
        cmd: media::MediaCmd,
    },
    /// Audio analysis: levels, waveform, silence, loudness.
    Audio {
        #[command(subcommand)]
        cmd: analyze::AudioCmd,
    },
    /// Render composited timeline frames (PNG files, a contact sheet, or brightness stats).
    Look {
        project: String,
        /// Times in seconds, comma-separated (default: --count frames across the timeline).
        #[arg(long)]
        at: Option<String>,
        /// One frame every N seconds.
        #[arg(long)]
        every: Option<f64>,
        #[arg(long, default_value_t = 6)]
        count: u32,
        #[arg(long, default_value_t = 640)]
        width: u32,
        /// Folder for one PNG per frame.
        #[arg(long)]
        out: Option<PathBuf>,
        /// One contact-sheet PNG.
        #[arg(long)]
        sheet: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        columns: u32,
        /// Mean brightness and black / clipped pixel shares per frame.
        #[arg(long)]
        stats: bool,
    },
    /// Render the frame under the running app's playhead.
    Screenshot {
        out: PathBuf,
        #[arg(long, default_value_t = 1280)]
        width: u32,
    },
    /// AI generation with your own provider keys: image, video, voice, music, sfx, status, models, key.
    Generate {
        #[command(subcommand)]
        cmd: generate::GenerateCmd,
    },
    /// The running app: context, open, seek, play, pause.
    App {
        #[command(subcommand)]
        cmd: app::AppCmd,
    },
    /// The timeline as text.
    Source {
        #[command(subcommand)]
        cmd: SourceCmd,
    },
    /// Report problems: overlaps, gaps, clips past their source, missing media, pending items.
    Verify { project: String },
    /// List revisions, oldest first.
    History { project: String },
    /// Revert the latest revision, or return to revision REV, as a new revision.
    Undo { project: String, rev: Option<i64> },
    /// Render the timeline to an MP4 file (H.264 + AAC). With --json, progress is printed as JSON lines.
    Export {
        project: String,
        out: PathBuf,
        /// Output height in pixels; the width follows the aspect ratio.
        #[arg(long, default_value_t = 1080)]
        height: u32,
        #[arg(long, default_value_t = 30)]
        fps: u32,
    },
    /// Exchange the timeline with other editors (FCPXML, Premiere/FCP7 XML, EDL, OpenTimelineIO).
    Timeline {
        #[command(subcommand)]
        cmd: TimelineCmd,
    },
    /// Built-in documentation (`ove docs agents` is the guide for agents).
    Docs { topic: Option<String> },
    /// Check the library, media support, encoders, the running app and agent setup.
    Doctor,
    /// Agent skills for Claude Code and Codex.
    Skills {
        #[command(subcommand)]
        cmd: SkillsCmd,
    },
    /// The supported AI agents installed on this computer (Claude, Codex, OpenCode, Hermes, Kimi, Grok).
    Agents {
        #[command(subcommand)]
        cmd: Option<AgentsCmd>,
    },
    /// Serve the commands as MCP tools on stdin/stdout (`claude mcp add openvideoeditor -- ove mcp`).
    Mcp,
}

#[derive(Subcommand)]
enum AgentsCmd {
    /// Which agents are installed, and their versions.
    List,
    /// Give an installed agent the editing guide and this program's MCP server (`ove agents connect claude`).
    Connect { agent: String },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// Write the openvideoeditor skill into the Claude Code / Codex skills folders (or --dir).
    Install {
        #[arg(long)]
        dir: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TimelineCmd {
    /// Write the timeline to FILE. The format comes from --format or the extension (.fcpxml .xml .edl .otio).
    Export {
        project: String,
        file: PathBuf,
        #[arg(long)]
        format: Option<String>,
        /// Frame rate of the exchange file (29.97 and 23.976 allowed).
        #[arg(long, default_value_t = 30.0)]
        fps: f64,
    },
    /// Add the tracks and clips of FILE to the project as one revision (new media awaits probing; clips whose media
    /// cannot be found become pending placeholders).
    Import {
        project: String,
        file: PathBuf,
        #[arg(long)]
        format: Option<String>,
        #[arg(long, default_value_t = 30.0)]
        fps: f64,
    },
}

#[derive(Subcommand)]
enum SourceCmd {
    /// Print the projection of the current revision.
    Print {
        project: String,
        /// First line to print (1-based).
        #[arg(long)]
        from: Option<usize>,
        /// Number of lines to print.
        #[arg(long)]
        lines: Option<usize>,
    },
    /// Check a projection file (default: the bundle's timeline.tsx; `-` reads stdin) without applying it.
    Check {
        project: String,
        file: Option<String>,
        #[arg(long, value_enum, default_value_t = Side::File)]
        prefer: Side,
    },
    /// Apply a projection file onto the current document as one revision.
    Apply {
        project: String,
        file: Option<String>,
        #[arg(long, value_enum, default_value_t = Side::File)]
        prefer: Side,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Side {
    File,
    Editor,
}

fn read_input(bundle: &Path, file: Option<&str>) -> Result<String, Fail> {
    let read = |p: &Path| {
        std::fs::read_to_string(p)
            .map_err(|e| Fail::new("file.unreadable", format!("could not read {}: {e}", p.display())))
    };
    match file {
        None => read(&bundle.join(PROJECTION_FILE)),
        Some("-") => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| Fail::new("file.unreadable", format!("could not read stdin: {e}")))?;
            Ok(s)
        }
        Some(f) => read(Path::new(f)),
    }
}

fn side(s: Side) -> &'static str {
    match s {
        Side::File => "file",
        Side::Editor => "editor",
    }
}

/// Runs a parsed command. `source` records where edits come from.
fn run(cli: &Cli, source: Source) -> Out {
    // Commands that need no library.
    match &cli.cmd {
        Cmd::Docs { topic } => {
            let r = ove_tools::tools::docs::page(topic.as_deref())?;
            return Ok((r.json, r.text));
        }
        Cmd::Doctor => return docs::doctor(cli.library.as_deref()),
        Cmd::Skills { cmd: SkillsCmd::Install { dir } } => return docs::install_skills(dir),
        Cmd::Agents { cmd: None | Some(AgentsCmd::List) } => return docs::agents_list(),
        Cmd::Agents { cmd: Some(AgentsCmd::Connect { agent }) } => return docs::agents_connect(agent),
        Cmd::Mcp => return Err(Fail::new("mcp.nested", "already serving MCP")),
        _ => {}
    }
    let ctx = Ctx::open(cli.library.as_deref(), source, cli.json, true)?;
    match &cli.cmd {
        Cmd::Project { cmd } => project::run(&ctx, cmd),
        Cmd::Media { cmd } => media::run(&ctx, cmd),
        Cmd::App { cmd } => app::run(&ctx, cmd),
        Cmd::Audio { cmd } => analyze::audio(&ctx, cmd),
        Cmd::Generate { cmd } => generate::run(&ctx, cmd),
        Cmd::Look { project, at, every, count, width, out, sheet, columns, stats } => ctx.tool(
            "look",
            json!({"project": project, "at": at, "every": every, "count": count, "width": width, "out": out,
                   "sheet": sheet, "columns": columns, "stats": stats}),
        ),
        Cmd::Screenshot { out, width } => ctx.tool("screenshot", json!({"out": out, "width": width})),
        Cmd::Source { cmd: SourceCmd::Print { project, from, lines } } => {
            ctx.tool("print_timeline", json!({"project": project, "from": from, "lines": lines}))
        }
        Cmd::Source { cmd: SourceCmd::Check { project, file, prefer } } => {
            let text = read_input(&ctx.find(project)?, file.as_deref())?;
            ctx.tool("check_timeline", json!({"project": project, "source": text, "prefer": side(*prefer)}))
        }
        Cmd::Source { cmd: SourceCmd::Apply { project, file, prefer } } => {
            let text = read_input(&ctx.find(project)?, file.as_deref())?;
            ctx.tool("apply_timeline", json!({"project": project, "source": text, "prefer": side(*prefer)}))
        }
        Cmd::Verify { project } => ctx.tool("verify_timeline", json!({"project": project})),
        Cmd::History { project } => ctx.tool("timeline_history", json!({"project": project})),
        Cmd::Undo { project, rev } => ctx.tool("undo_timeline", json!({"project": project, "rev": rev})),
        Cmd::Export { project, out, height, fps } => {
            ctx.tool("export_video", json!({"project": project, "out": out, "height": height, "fps": fps}))
        }
        Cmd::Timeline { cmd: TimelineCmd::Export { project, file, format, fps } } => {
            ctx.tool("export_timeline_file", json!({"project": project, "file": file, "format": format, "fps": fps}))
        }
        Cmd::Timeline { cmd: TimelineCmd::Import { project, file, format, fps } } => {
            ctx.tool("import_timeline_file", json!({"project": project, "file": file, "format": format, "fps": fps}))
        }
        // Answered before the library is opened.
        Cmd::Docs { .. } | Cmd::Doctor | Cmd::Skills { .. } | Cmd::Agents { .. } | Cmd::Mcp => {
            Err(Fail::new("internal", "this command does not use a library"))
        }
    }
}

/// Exit code for a refusal: 3 when the app is needed but not running, else 1.
fn exit_code(f: &Fail) -> u8 {
    if f.code == "app.notRunning" { 3 } else { 1 }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.cmd, Cmd::Mcp) {
        return match mcp::serve(cli.library.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }
    match run(&cli, Source::Cli) {
        Ok((v, text)) => {
            // A closed pipe (`ove … | head`) is not an error.
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let _ = if cli.json {
                writeln!(out, "{v}")
            } else if !text.is_empty() {
                writeln!(out, "{text}")
            } else {
                Ok(())
            };
            ExitCode::SUCCESS
        }
        Err(f) => {
            if cli.json {
                eprintln!("{}", json!({"error": {"code": f.code, "message": f.message, "details": f.details}}));
            } else {
                eprintln!("error: {}", f.message);
            }
            ExitCode::from(exit_code(&f))
        }
    }
}
