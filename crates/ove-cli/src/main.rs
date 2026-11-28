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
mod edit;
mod export;
mod generate;
mod markers;
mod mcp;
mod media;
mod model_worker;
mod models;
mod project;
mod skills;
mod versions;

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
        /// Compositing backend: auto (the GPU when its self-test passes), cpu or gpu.
        #[arg(long)]
        backend: Option<String>,
        /// result (default) or matte: every chroma key shows its matte.
        #[arg(long)]
        view: Option<String>,
        /// waveform, parade or vectorscope: write that scope instead of the frame (--out: a .png for one time, or a
        /// folder).
        #[arg(long)]
        scope: Option<String>,
    },
    /// Colour readouts for matching shots: inspect.
    Color {
        #[command(subcommand)]
        cmd: ColorCmd,
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
    /// Edit verbs: split, trim, ripple-trim, ripple-delete, delete, insert, overwrite, lift, extract, close-gap, slip,
    /// slide, roll, move. One revision each; --dry-run prints the plan only.
    Edit(Box<edit::EditArgs>),
    /// Report problems: overlaps, gaps, clips past their source, missing media, pending items.
    Verify { project: String },
    /// Timeline markers: list (by status and assignee), add, update, remove.
    Markers {
        #[command(subcommand)]
        cmd: markers::MarkersCmd,
    },
    /// Named versions: save, list, print, diff, restore.
    Version {
        #[command(subcommand)]
        cmd: versions::VersionCmd,
    },
    /// List revisions, oldest first.
    History { project: String },
    /// The words said on the timeline (with word ids and timeline times), transcript search, fillers and
    /// transcription. `ove transcript <project>` prints the words.
    Transcript(TranscriptArgs),
    /// Revert the latest revision, or return to revision REV, as a new revision. With --any and --session (or
    /// --turn), revert only that agent session's (or turn's) changes, keeping everyone else's.
    Undo {
        project: String,
        rev: Option<i64>,
        /// Undo another actor's changes: the session (and turn) named by --session / --turn.
        #[arg(long, requires = "session")]
        any: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        turn: Option<String>,
        /// The session's actor kind (mcp, assistant, cli).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Render the timeline (or a range) to a video or audio file: a preset, or a format with codec options (see
    /// `ove docs export`). With --json, progress is printed as JSON lines. `ove render` is the same command.
    #[command(alias = "render")]
    Export(export::ExportCmd),
    /// Exchange the timeline with other editors (FCPXML, Premiere/FCP7 XML, EDL, OpenTimelineIO).
    Timeline {
        #[command(subcommand)]
        cmd: TimelineCmd,
    },
    /// Background tasks of the running app (thumbnails, skim sheets, proxies, captions, export): list, wait, cancel.
    Tasks {
        #[command(subcommand)]
        cmd: TasksCmd,
    },
    /// Built-in documentation (`ove docs agents` is the guide for agents).
    Docs { topic: Option<String> },
    /// Check the library, media support, encoders, the running app and agent setup.
    Doctor,
    /// Skills: playbooks agents read (the library's skill store), and the editing guide for Claude Code and Codex.
    Skills {
        #[command(subcommand)]
        cmd: skills::SkillsCmd,
    },
    /// The supported AI agents installed on this computer (Claude, Codex, OpenCode, Hermes, Kimi, Grok).
    Agents {
        #[command(subcommand)]
        cmd: Option<AgentsCmd>,
    },
    /// Local machine-learning models: list, install (models over 50 MB need --yes), remove.
    Models {
        #[command(subcommand)]
        cmd: models::ModelsCmd,
    },
    /// Runs models for the app in a separate process (started by the app; binary protocol on stdin/stdout).
    #[command(hide = true)]
    ModelWorker {
        /// Exit after this many seconds without a job.
        #[arg(long, default_value_t = 120)]
        idle_secs: u64,
    },
    /// Serve the commands as MCP tools on stdin/stdout (`claude mcp add openvideoeditor -- ove mcp`).
    Mcp {
        /// Bind the session to this project: every tool works on it, and calls naming another are refused.
        #[arg(long)]
        project: Option<String>,
        /// Record this turn id on every revision the session makes (for `undo_my_changes` scope turn).
        #[arg(long)]
        turn: Option<String>,
    },
}

#[derive(Subcommand)]
enum ColorCmd {
    /// Measure the colour of the exported frame at a time (or of one item alone), optionally against a reference
    /// item or media frame with a suggested grade patch.
    Inspect {
        project: String,
        /// Timeline time in seconds (snapped to the frame grid; default: the item's middle).
        #[arg(long)]
        at: Option<f64>,
        /// The shot to measure and to grade.
        #[arg(long)]
        item: Option<String>,
        /// Measure the item alone, without the layers above or below it.
        #[arg(long)]
        isolate: bool,
        /// Item id or media id/file name to match.
        #[arg(long)]
        reference: Option<String>,
        /// Timeline time of a reference item (default its middle), or source time of reference media (default 0).
        #[arg(long)]
        reference_at: Option<f64>,
        /// Share of the fitted correction to suggest, 0..1 (each step is capped).
        #[arg(long)]
        strength: Option<f64>,
        /// Measuring height in pixels (default 1080, the export default).
        #[arg(long)]
        height: Option<u32>,
        /// Compositing backend: auto (the GPU when its self-test passes), cpu or gpu.
        #[arg(long)]
        backend: Option<String>,
    },
}

#[derive(Subcommand)]
enum TasksCmd {
    /// Unfinished background tasks, running first (none when the app is not running).
    List {
        /// Only this project's tasks.
        project: Option<String>,
        /// Only this kind (thumbnail, skim, proxy, captions, export, audio-analysis, …).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Wait until the matching tasks are done, or the time-out; prints whether they are done and how many remain.
    Wait {
        project: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        /// Seconds to wait at most (at most 600).
        #[arg(long, default_value_t = 30.0)]
        timeout: f64,
    },
    /// Cancel a task by id (from `ove tasks list`).
    Cancel { id: i64 },
}

#[derive(Subcommand)]
enum AgentsCmd {
    /// Which agents are installed, and their versions.
    List,
    /// Give an installed agent the editing guide and this program's MCP server (`ove agents connect claude`).
    Connect { agent: String },
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
        Cmd::Skills { cmd } => return skills::run(cmd, cli.library.as_deref(), source),
        Cmd::Agents { cmd: None | Some(AgentsCmd::List) } => return docs::agents_list(),
        Cmd::Agents { cmd: Some(AgentsCmd::Connect { agent }) } => return docs::agents_connect(agent),
        Cmd::Models { cmd } => return models::run(cmd, source),
        Cmd::Export(c) if c.args.worker => return export::worker(),
        Cmd::ModelWorker { .. } => return Err(Fail::new("argument.invalid", "the model worker is started by the app")),
        Cmd::Mcp { .. } => return Err(Fail::new("mcp.nested", "already serving MCP")),
        _ => {}
    }
    let ctx = Ctx::open(cli.library.as_deref(), source, cli.json, true)?;
    match &cli.cmd {
        Cmd::Project { cmd } => project::run(&ctx, cmd),
        Cmd::Media { cmd } => media::run(&ctx, cmd),
        Cmd::App { cmd } => app::run(&ctx, cmd),
        Cmd::Audio { cmd } => analyze::audio(&ctx, cmd),
        Cmd::Generate { cmd } => generate::run(&ctx, cmd),
        Cmd::Look { project, at, every, count, width, out, sheet, columns, stats, backend, view, scope } => ctx.tool(
            "look",
            json!({"project": project, "at": at, "every": every, "count": count, "width": width, "out": out,
                   "sheet": sheet, "columns": columns, "stats": stats, "backend": backend, "view": view,
                   "scope": scope}),
        ),
        Cmd::Color {
            cmd: ColorCmd::Inspect { project, at, item, isolate, reference, reference_at, strength, height, backend },
        } => ctx.tool(
            "inspect_color",
            json!({"project": project, "at": at, "item": item, "isolate": isolate, "reference": reference,
                   "referenceAt": reference_at, "strength": strength, "height": height, "backend": backend}),
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
        Cmd::Edit(a) => edit::run(&ctx, a),
        Cmd::Markers { cmd } => markers::run(&ctx, cmd),
        Cmd::Version { cmd } => versions::run(&ctx, cmd),
        Cmd::Verify { project } => ctx.tool("verify_timeline", json!({"project": project})),
        Cmd::History { project } => ctx.tool("timeline_history", json!({"project": project})),
        Cmd::Transcript(t) => t.run(&ctx),
        Cmd::Undo { project, rev, any, session, turn, kind } => {
            if session.is_some() || turn.is_some() {
                if !any {
                    return Err(Fail::new(
                        "argument.invalid",
                        "--session and --turn undo another actor's changes: add --any",
                    ));
                }
                ctx.tool(
                    "undo_my_changes",
                    json!({"project": project, "scope": "any", "session": session, "turn": turn, "kind": kind}),
                )
            } else {
                ctx.tool("undo_timeline", json!({"project": project, "rev": rev}))
            }
        }
        Cmd::Export(a) => export::run(&ctx, a),
        Cmd::Tasks { cmd: TasksCmd::List { project, kind } } => {
            ctx.tool("list_tasks", json!({"project": project, "kind": kind}))
        }
        Cmd::Tasks { cmd: TasksCmd::Wait { project, kind, timeout } } => {
            ctx.tool("wait_tasks", json!({"project": project, "kind": kind, "timeoutSeconds": timeout}))
        }
        Cmd::Tasks { cmd: TasksCmd::Cancel { id } } => ctx.tool("cancel_task", json!({"id": id})),
        Cmd::Timeline { cmd: TimelineCmd::Export { project, file, format, fps } } => {
            ctx.tool("export_timeline_file", json!({"project": project, "file": file, "format": format, "fps": fps}))
        }
        Cmd::Timeline { cmd: TimelineCmd::Import { project, file, format, fps } } => {
            ctx.tool("import_timeline_file", json!({"project": project, "file": file, "format": format, "fps": fps}))
        }
        // Answered before the library is opened.
        Cmd::Docs { .. }
        | Cmd::Doctor
        | Cmd::Skills { .. }
        | Cmd::Agents { .. }
        | Cmd::Models { .. }
        | Cmd::ModelWorker { .. }
        | Cmd::Mcp { .. } => Err(Fail::new("internal", "this command does not use a library")),
    }
}

/// Exit code for a refusal: 3 when the app is needed but not running, else 1.
fn exit_code(f: &Fail) -> u8 {
    if f.code == "app.notRunning" { 3 } else { 1 }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Queued exports and other renders pick the compositing backend through this (design D9).
    #[cfg(feature = "media")]
    ove_tools::tools::backend::install_provider();
    if let Cmd::ModelWorker { idle_secs } = cli.cmd {
        return model_worker::main(idle_secs);
    }
    if let Cmd::Mcp { project, turn } = &cli.cmd {
        return match mcp::serve(cli.library.as_deref(), project.as_deref(), turn.as_deref()) {
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

/// `ove transcript …` (add-transcript-editing).
#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
struct TranscriptArgs {
    #[command(subcommand)]
    cmd: Option<TranscriptCmd>,
    /// The project.
    project: Option<String>,
    /// Only these clip ids (comma separated).
    #[arg(long, value_delimiter = ',')]
    clips: Vec<String>,
    /// Sentences instead of words.
    #[arg(long)]
    segments: bool,
    /// First word of the page.
    #[arg(long)]
    offset: Option<i64>,
    /// Words per page (at most 10000).
    #[arg(long)]
    limit: Option<i64>,
}

#[derive(clap::Subcommand)]
enum TranscriptCmd {
    /// Find a phrase in everything said in the project's transcribed media.
    Search { project: String, phrase: String },
    /// Filler candidates for review (nothing is cut).
    Fillers { project: String },
    /// Recognise a media file's speech on this computer and cache it in the project.
    Transcribe {
        project: String,
        media: String,
        /// tiny, base (default) or small.
        #[arg(long)]
        model: Option<String>,
        /// ISO 639-1 code (default: detect).
        #[arg(long)]
        language: Option<String>,
        /// Keep hesitations (um, uh).
        #[arg(long)]
        verbatim: bool,
        /// Transcribe again even when cached.
        #[arg(long)]
        force: bool,
    },
}

impl TranscriptArgs {
    fn run(&self, ctx: &common::Ctx) -> common::Out {
        match &self.cmd {
            Some(TranscriptCmd::Search { project, phrase }) => {
                ctx.tool("search_transcript", json!({"project": project, "phrase": phrase}))
            }
            Some(TranscriptCmd::Fillers { project }) => ctx.tool("find_fillers", json!({"project": project})),
            Some(TranscriptCmd::Transcribe { project, media, model, language, verbatim, force }) => ctx.tool(
                "transcribe_media",
                json!({"project": project, "media": media, "model": model, "language": language,
                    "verbatim": verbatim, "force": force}),
            ),
            None => {
                let project = self.project.clone().ok_or_else(|| {
                    common::Fail::new("argument.missing", "`ove transcript` needs a project (or `search`, `fillers`)")
                })?;
                let mut a =
                    json!({"project": project, "segments": self.segments, "offset": self.offset, "limit": self.limit});
                if !self.clips.is_empty() {
                    a["clips"] = json!(self.clips);
                }
                ctx.tool("get_transcript", a)
            }
        }
    }
}
