//! `ove` (working name): the command-line tool. This change provides the timeline commands — `source print`,
//! `source check`, `source apply`, `verify`, `history`, `undo` — working headless on project bundles.
//! Results go to stdout (one JSON object with `--json`), errors to stderr. Exit codes: 0 ok, 1 refused, 2 usage.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use ove_core::revision::Source;
use ove_projection::verify::{Context, Registry, has_errors};
use ove_projection::{ApplyInput, BaseError, Plan, Prefer, Refusal};
use ove_store::media::{MediaState, media_status};
use ove_store::project::{Mode, PROJECTION_FILE, Project};
use ove_store::{Library, StoreError};
use serde_json::{Value, json};

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

/// A refusal: code, message and optional details for JSON.
struct Fail {
    code: String,
    message: String,
    details: Value,
}

impl From<StoreError> for Fail {
    fn from(e: StoreError) -> Self {
        Fail { code: e.code().to_string(), message: e.to_string(), details: Value::Null }
    }
}

impl From<Refusal> for Fail {
    fn from(r: Refusal) -> Self {
        Fail { code: r.code.clone(), message: r.to_string(), details: json!({"diagnostics": r.diagnostics}) }
    }
}

type Out = Result<(Value, String), Fail>;

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

fn library(cli: &Cli) -> Result<Library, Fail> {
    let root = match &cli.library {
        Some(p) => {
            ove_store::library::validate_library(p)?;
            p.clone()
        }
        None => ove_store::library::resolve_library(None)?,
    };
    Ok(Library::open(root, clock())?)
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

fn run(cli: &Cli) -> Out {
    let lib = library(cli)?;
    match &cli.cmd {
        Cmd::Source { cmd: SourceCmd::Print { project, from, lines } } => {
            let p = Project::open(&lib.find(project)?, Mode::ReadOnly, clock())?;
            let full = p.projection();
            let text = ove_projection::print::slice_lines(&full, from.unwrap_or(1), *lines);
            Ok((json!({"rev": p.rev(), "text": text}), text.trim_end_matches('\n').to_string()))
        }
        Cmd::Source { cmd: SourceCmd::Check { project, file, prefer } } => {
            let p = Project::open(&lib.find(project)?, Mode::ReadOnly, clock())?;
            let text = read_input(p.bundle(), file.as_deref())?;
            let plan = plan_for(&p, &text, (*prefer).into())?;
            let mut v = plan_json(&plan);
            v["ok"] = json!(true);
            Ok((v, format!("ok: {}", plan_text(&plan))))
        }
        Cmd::Source { cmd: SourceCmd::Apply { project, file, prefer } } => {
            let mut p = Project::open(&lib.find(project)?, Mode::Write, clock())?;
            let text = read_input(p.bundle(), file.as_deref())?;
            let plan = plan_for(&p, &text, (*prefer).into())?;
            let mut v = plan_json(&plan);
            let changed = !plan.ops.is_empty();
            if changed {
                let summary = format!("source apply: {}", plan.summary());
                p.apply(plan.ops.clone(), Source::Cli, Some(summary))?;
            }
            p.set_diagnostics(plan.warnings.iter().map(|w| w.to_string()).collect())?;
            p.flush_projection();
            v["rev"] = json!(p.rev());
            v["changed"] = json!(changed);
            let text = if changed {
                format!("rev {}: {}", p.rev(), plan_text(&plan))
            } else {
                format!("no changes (rev {})", p.rev())
            };
            p.close()?;
            Ok((v, text))
        }
        Cmd::Verify { project } => {
            let p = Project::open(&lib.find(project)?, Mode::ReadOnly, clock())?;
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
                return Err(Fail { code: "verify.failed".into(), message: text, details: v });
            }
            Ok((v, text))
        }
        Cmd::History { project } => {
            let p = Project::open(&lib.find(project)?, Mode::ReadOnly, clock())?;
            let h = p.history()?;
            let text = h
                .iter()
                .map(|e| format!("{}\t{}\t{}", e.rev, e.source.as_str(), e.summary))
                .collect::<Vec<_>>()
                .join("\n");
            Ok((json!({"rev": p.rev(), "revisions": h}), text))
        }
        Cmd::Undo { project, rev } => {
            let mut p = Project::open(&lib.find(project)?, Mode::Write, clock())?;
            let target = match rev {
                Some(r) => *r,
                None => match p.latest_record()?.filter(|r| r.rev == p.rev()) {
                    Some(r) => r.parent,
                    None => {
                        return Err(Fail {
                            code: "undo.nothing".into(),
                            message: "nothing to undo".into(),
                            details: Value::Null,
                        });
                    }
                },
            };
            let c = p.restore_as(target, Source::Undo)?;
            p.flush_projection();
            let v = json!({"rev": p.rev(), "changed": c.is_some(), "target": target});
            let text = match c {
                Some(c) => format!("rev {}: {}", c.record.rev, c.record.summary),
                None => format!("already at the state of revision {target}"),
            };
            p.close()?;
            Ok((v, text))
        }
        Cmd::Export { project, out, height, fps } => export(&lib, project, out, *height, *fps, cli.json),
    }
}

#[cfg(feature = "export")]
fn export(lib: &Library, project: &str, out: &Path, height: u32, fps: u32, json_lines: bool) -> Out {
    use std::io::Write;
    let p = Project::open(&lib.find(project)?, Mode::ReadOnly, clock())?;
    let settings = ove_render::ExportSettings { height, fps, ..Default::default() };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = -1i64;
    let res = ove_render::export(p.doc(), Some(p.bundle()), out, &settings, &cancel, |pr| {
        let pct = (pr.fraction * 100.0) as i64;
        if pct != last {
            last = pct;
            if json_lines {
                println!("{}", json!({"progress": pr}));
            } else {
                eprint!("\rexporting {pct}%");
            }
            let _ = std::io::stdout().flush();
        }
    })
    .map_err(|e| Fail {
        code: match e {
            ove_render::ExportError::Refused(_) => "export.refused".into(),
            ove_render::ExportError::Cancelled => "export.cancelled".into(),
            ove_render::ExportError::Ffmpeg(_) => "export.failed".into(),
        },
        message: e.to_string(),
        details: Value::Null,
    })?;
    if !json_lines {
        eprintln!();
    }
    let text = format!(
        "exported {} ({}x{}, {:.1} s, {}) in {:.1} s",
        res.path, res.width, res.height, res.duration_seconds, res.video_encoder, res.seconds_taken
    );
    Ok((serde_json::to_value(&res).unwrap(), text))
}

#[cfg(not(feature = "export"))]
fn export(_: &Library, _: &str, _: &Path, _: u32, _: u32, _: bool) -> Out {
    Err(Fail {
        code: "export.unavailable".into(),
        message: "this build of ove has no export; build it with `--features export`".into(),
        details: Value::Null,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok((v, text)) => {
            if cli.json {
                println!("{v}");
            } else if !text.is_empty() {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(f) => {
            if cli.json {
                eprintln!("{}", json!({"error": {"code": f.code, "message": f.message, "details": f.details}}));
            } else {
                eprintln!("error: {}", f.message);
            }
            ExitCode::from(1)
        }
    }
}
