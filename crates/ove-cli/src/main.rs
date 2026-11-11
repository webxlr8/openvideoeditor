//! `ove`: the command line for OpenVideoEditor projects. Works on project bundles with the app closed; when the app
//! has a project open, edits go through it (control link) so the editor updates at once. `ove mcp` serves the same
//! commands as MCP tools. Results go to stdout (one JSON object with `--json`), errors to stderr. Exit codes: 0 ok,
//! 1 refused, 2 usage, 3 the app is not running (app commands).

// Without media support some options and helpers are parsed but unused.
#![cfg_attr(not(feature = "media"), allow(dead_code))]

mod analyze;
mod app;
mod common;
mod docs;
mod mcp;
mod media;
mod project;

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use ove_core::revision::Source;
use ove_projection::verify::{Context, Registry, has_errors};
use ove_projection::{ApplyInput, BaseError, Plan, Prefer, Refusal};
use ove_store::media::{MediaState, media_status};
use ove_store::project::{PROJECTION_FILE, Project};
use serde_json::{Value, json};

use common::{Ctx, Fail, Out, edit, via};

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
    /// Serve the commands as MCP tools on stdin/stdout (`claude mcp add openvideoeditor -- ove mcp`).
    Mcp,
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

fn interchange_format(file: &std::path::Path, format: &Option<String>) -> Result<ove_interchange::Format, Fail> {
    match format {
        Some(f) => ove_interchange::Format::parse(f),
        None => ove_interchange::Format::from_path(file),
    }
    .map_err(|e| Fail::new("format.unknown", e.to_string()))
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

impl From<Side> for Prefer {
    fn from(s: Side) -> Prefer {
        match s {
            Side::File => Prefer::File,
            Side::Editor => Prefer::Editor,
        }
    }
}

fn read_input(bundle: &Path, file: Option<&str>) -> Result<String, Fail> {
    let read = |p: &Path| {
        std::fs::read_to_string(p).map_err(|e| Fail {
            code: "file.unreadable".into(),
            message: format!("could not read {}: {e}", p.display()),
            details: Value::Null,
        })
    };
    match file {
        None => read(&bundle.join(PROJECTION_FILE)),
        Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).map_err(|e| Fail {
                code: "file.unreadable".into(),
                message: format!("could not read stdin: {e}"),
                details: Value::Null,
            })?;
            Ok(s)
        }
        Some(f) => read(Path::new(f)),
    }
}

fn plan_for(p: &Project, text: &str, prefer: Prefer) -> Result<Plan, Refusal> {
    let base_for = |rev: i64| {
        p.doc_at(rev).map_err(|e| BaseError {
            code: match e.code() {
                "revision.missing" => "rev.pruned".into(),
                "revision.newer" => "rev.newer".into(),
                c => c.to_string(),
            },
            message: format!("the projection's base revision {rev} is not available ({e})"),
        })
    };
    ove_projection::plan(&ApplyInput {
        text,
        current: p.doc(),
        base_for: &base_for,
        bundle: Some(p.bundle()),
        prefer,
        now_ms: ove_core::Clock::now_ms(&ove_core::SystemClock),
    })
}

fn plan_json(p: &Plan) -> Value {
    json!({
        "baseRev": p.base_rev,
        "added": p.counts.added,
        "updated": p.counts.updated,
        "removed": p.removed.len(),
        "removedItems": p.removed,
        "conflicts": p.conflicts,
        "warnings": p.warnings,
    })
}

fn plan_text(p: &Plan) -> String {
    let mut s = p.summary();
    for c in &p.conflicts {
        s.push_str(&format!(
            "\nconflict: {} (kept the {} value)",
            c.message,
            match c.kept {
                Prefer::File => "file",
                Prefer::Editor => "editor",
            }
        ));
    }
    for w in &p.warnings {
        s.push_str(&format!("\n{w}"));
    }
    s
}

/// Runs a parsed command. `source` records where edits come from (cli or mcp).
fn run(cli: &Cli, source: Source) -> Out {
    // Commands that need no library.
    match &cli.cmd {
        Cmd::Docs { topic } => return docs::docs(topic.as_deref()),
        Cmd::Doctor => return docs::doctor(cli.library.as_deref()),
        Cmd::Skills { cmd: SkillsCmd::Install { dir } } => return docs::install_skills(dir),
        Cmd::Mcp => return Err(Fail::new("mcp.nested", "already serving MCP")),
        _ => {}
    }
    let ctx = Ctx::open(cli.library.as_deref(), source, cli.json)?;
    match &cli.cmd {
        Cmd::Project { cmd } => project::run(&ctx, cmd),
        Cmd::Media { cmd } => media::run(&ctx, cmd),
        Cmd::App { cmd } => app::run(&ctx, cmd),
        Cmd::Audio { cmd } => analyze::audio(&ctx, cmd),
        Cmd::Look { project, at, every, count, width, out, sheet, columns, stats } => analyze::look(
            &ctx,
            project,
            &analyze::LookOpts {
                at: at.as_deref(),
                every: *every,
                count: *count,
                width: *width,
                out: out.as_deref(),
                sheet: sheet.as_deref(),
                columns: *columns,
                stats: *stats,
            },
        ),
        Cmd::Screenshot { out, width } => analyze::screenshot(&ctx, out, *width),
        Cmd::Source { cmd: SourceCmd::Print { project, from, lines } } => {
            let p = ctx.read(project)?;
            let full = p.projection();
            let text = ove_projection::print::slice_lines(&full, from.unwrap_or(1), *lines);
            Ok((json!({"rev": p.rev(), "text": text}), text.trim_end_matches('\n').to_string()))
        }
        Cmd::Source { cmd: SourceCmd::Check { project, file, prefer } } => {
            let p = ctx.read(project)?;
            let text = read_input(p.bundle(), file.as_deref())?;
            let plan = plan_for(&p, &text, (*prefer).into())?;
            let mut v = plan_json(&plan);
            v["ok"] = json!(true);
            Ok((v, format!("ok: {}", plan_text(&plan))))
        }
        Cmd::Source { cmd: SourceCmd::Apply { project, file, prefer } } => edit(&ctx, project, |w| {
            let text = read_input(w.bundle(), file.as_deref())?;
            let plan = plan_for(w.project(), &text, (*prefer).into())?;
            let mut v = plan_json(&plan);
            let changed = !plan.ops.is_empty();
            if changed {
                let summary = format!("source apply: {}", plan.summary());
                w.apply(plan.ops.clone(), ctx.source, summary)?;
            }
            w.set_diagnostics(plan.warnings.iter().map(|w| w.to_string()).collect())?;
            v["rev"] = json!(w.rev());
            v["changed"] = json!(changed);
            v["viaApp"] = json!(w.via_app());
            let text = if changed {
                format!("rev {}: {}{}", w.rev(), plan_text(&plan), via(w))
            } else {
                format!("no changes (rev {})", w.rev())
            };
            Ok((v, text))
        }),
        Cmd::Verify { project } => {
            let p = ctx.read(project)?;
            let offline: BTreeSet<String> = media_status(p.bundle(), p.doc())
                .into_iter()
                .filter(|m| m.state == MediaState::Offline)
                .map(|m| m.media_id)
                .collect();
            let findings = Registry::default().run(&Context { doc: p.doc(), offline: &offline });
            let text = if findings.is_empty() {
                "0 findings".to_string()
            } else {
                findings
                    .iter()
                    .map(|f| {
                        format!(
                            "{}\t{}\t{}",
                            match f.severity {
                                ove_projection::verify::Level::Error => "error",
                                _ => "warning",
                            },
                            f.lint,
                            f.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let v = json!({"rev": p.rev(), "findings": findings, "count": findings.len()});
            if has_errors(&findings) {
                return Err(Fail::new("verify.failed", text).details(v));
            }
            Ok((v, text))
        }
        Cmd::History { project } => {
            let p = ctx.read(project)?;
            let h = p.history()?;
            let text = h
                .iter()
                .map(|e| format!("{}\t{}\t{}", e.rev, e.source.as_str(), e.summary))
                .collect::<Vec<_>>()
                .join("\n");
            Ok((json!({"rev": p.rev(), "revisions": h}), text))
        }
        Cmd::Undo { project, rev } => edit(&ctx, project, |w| {
            let target = match rev {
                Some(r) => *r,
                None => match w.project().latest_record()?.filter(|r| r.rev == w.rev()) {
                    Some(r) => r.parent,
                    None => return Err(Fail::new("undo.nothing", "nothing to undo")),
                },
            };
            let c = w.restore(target, Source::Undo)?;
            let v = json!({"rev": w.rev(), "changed": c.is_some(), "target": target, "viaApp": w.via_app()});
            let text = match c {
                Some((r, summary)) => format!("rev {r}: {summary}{}", via(w)),
                None => format!("already at the state of revision {target}"),
            };
            Ok((v, text))
        }),
        Cmd::Export { project, out, height, fps } => export(&ctx, project, out, *height, *fps),
        Cmd::Timeline { cmd: TimelineCmd::Export { project, file, format, fps } } => {
            let f = interchange_format(file, format)?;
            let p = ctx.read(project)?;
            let text = ove_interchange::export(p.doc(), Some(p.bundle()), f, ove_interchange::Rate::from_fps(*fps));
            std::fs::write(file, &text)
                .map_err(|e| Fail::new("file.unwritable", format!("could not write {}: {e}", file.display())))?;
            Ok((
                json!({"path": file, "format": f.extension(), "bytes": text.len()}),
                format!("wrote {} ({})", file.display(), f.label()),
            ))
        }
        Cmd::Timeline { cmd: TimelineCmd::Import { project, file, format, fps } } => {
            let f = interchange_format(file, format)?;
            let text = std::fs::read_to_string(file)
                .map_err(|e| Fail::new("file.unreadable", format!("could not read {}: {e}", file.display())))?;
            let tl = ove_interchange::read(&text, f, ove_interchange::Rate::from_fps(*fps))
                .map_err(|e| Fail::new("import.invalid", format!("{}: {e}", file.display())))?;
            edit(&ctx, project, |w| {
                let (ops, summary) = ove_interchange::to_ops(w.doc(), &tl, common::now_ms());
                if ops.is_empty() {
                    return Err(Fail::new("import.empty", format!("{} has no clips", file.display())));
                }
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let rev = w.apply(ops, ctx.source, format!("import {name} ({} clips)", summary.clips))?;
                let v = json!({"rev": rev, "tracks": summary.tracks, "clips": summary.clips, "mediaAdded": summary.media_added, "missing": summary.missing});
                let text = format!(
                    "rev {rev}: imported {} clips on {} tracks ({} new media{}){}",
                    summary.clips,
                    summary.tracks,
                    summary.media_added,
                    if summary.missing.is_empty() {
                        String::new()
                    } else {
                        format!("; missing: {}", summary.missing.join(", "))
                    },
                    via(w)
                );
                Ok((v, text))
            })
        }
        Cmd::Docs { .. } | Cmd::Doctor | Cmd::Skills { .. } | Cmd::Mcp => unreachable!("handled above"),
    }
}

#[cfg(feature = "media")]
fn export(ctx: &Ctx, project: &str, out: &Path, height: u32, fps: u32) -> Out {
    use std::io::Write;
    let p = ctx.read(project)?;
    let settings = ove_render::ExportSettings { height, fps, ..Default::default() };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = -1i64;
    let show = common::progress_enabled();
    let res = ove_render::export(p.doc(), Some(p.bundle()), out, &settings, &cancel, |pr| {
        let pct = (pr.fraction * 100.0) as i64;
        if pct != last && show {
            last = pct;
            if ctx.json {
                println!("{}", json!({"progress": pr}));
            } else {
                eprint!("\rexporting {pct}%");
            }
            let _ = std::io::stdout().flush();
        }
    })
    .map_err(|e| {
        Fail::new(
            match e {
                ove_render::ExportError::Refused(_) => "export.refused",
                ove_render::ExportError::Cancelled => "export.cancelled",
                ove_render::ExportError::Ffmpeg(_) => "export.failed",
            },
            e.to_string(),
        )
    })?;
    if !ctx.json && show {
        eprintln!();
    }
    let text = format!(
        "exported {} ({}x{}, {:.1} s, {}) in {:.1} s",
        res.path, res.width, res.height, res.duration_seconds, res.video_encoder, res.seconds_taken
    );
    Ok((serde_json::to_value(&res).unwrap(), text))
}

#[cfg(not(feature = "media"))]
fn export(_: &Ctx, _: &str, _: &Path, _: u32, _: u32) -> Out {
    Err(Fail::new("export.unavailable", "this build of ove has no export; build it with `--features media`"))
}

/// Exit code for a refusal: 3 when the app is needed but not running, else 1.
fn exit_code(f: &Fail) -> u8 {
    if f.code == "app.notRunning" { 3 } else { 1 }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.cmd, Cmd::Mcp) {
        let library = cli.library.clone();
        let runner = move |args: Vec<String>| -> Out {
            let mut argv = vec!["ove".to_string(), "--json".to_string()];
            if let Some(l) = &library {
                argv.push("--library".into());
                argv.push(l.display().to_string());
            }
            argv.extend(args);
            let parsed = Cli::try_parse_from(&argv).map_err(|e| Fail::new("argument.invalid", e.to_string()))?;
            if matches!(parsed.cmd, Cmd::Mcp) {
                return Err(Fail::new("mcp.nested", "already serving MCP"));
            }
            run(&parsed, Source::Mcp)
        };
        return match mcp::serve(&runner) {
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
