//! `ove-doc`: a small test CLI over the document store, used by later changes' tests (task 6.2).
//!
//! ```text
//! ove-doc [--library <dir>] [--json] create <name>
//! ove-doc [--library <dir>] [--json] list
//! ove-doc [--library <dir>] [--json] apply <project> <ops.json | -> [--source gui|cli|mcp|agent]
//! ove-doc [--library <dir>] [--json] history <project>
//! ove-doc [--library <dir>] [--json] undo <project>       # return to the state before the latest revision
//! ove-doc [--library <dir>] [--json] redo <project>       # if the latest revision is an undo, revert it
//! ove-doc [--library <dir>] [--json] restore <project> <rev>
//! ove-doc [--library <dir>] [--json] show <project>       # print the current project.json
//! ```
//! `<project>` is a name, id or bundle path. Exit codes: 0 ok, 1 refused, 2 usage.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::library::resolve_library;
use ove_store::project::{Mode, Project};
use ove_store::{Committed, Library, Listing, StoreError};
use serde_json::{Value, json};

struct Args {
    library: Option<PathBuf>,
    json: bool,
    rest: Vec<String>,
    source: Source,
}

fn parse() -> Result<Args, String> {
    let mut a = Args { library: None, json: false, rest: vec![], source: Source::Cli };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--library" => a.library = Some(it.next().ok_or("--library needs a path")?.into()),
            "--json" => a.json = true,
            "--source" => {
                a.source = match it.next().as_deref() {
                    Some("gui") => Source::Gui,
                    Some("cli") => Source::Cli,
                    Some("mcp") => Source::Mcp,
                    Some("agent") => Source::Agent,
                    other => return Err(format!("unknown source {other:?}")),
                }
            }
            "-h" | "--help" => return Err(String::new()),
            _ => a.rest.push(x),
        }
    }
    Ok(a)
}

const USAGE: &str = "usage: ove-doc [--library <dir>] [--json] <create NAME | list | apply PROJECT OPS.json|- [--source S] | history PROJECT | undo PROJECT | redo PROJECT | restore PROJECT REV | show PROJECT>";

fn committed_json(c: &Option<Committed>) -> Value {
    match c {
        None => json!({"changed": false}),
        Some(c) => json!({"changed": true, "rev": c.record.rev, "source": c.record.source, "summary": c.record.summary,
            "warnings": c.warnings.iter().map(|w| json!({"code": w.code, "path": w.path, "message": w.message})).collect::<Vec<_>>()}),
    }
}

fn run(a: Args) -> Result<(Value, String), StoreError> {
    let clock = Arc::new(ove_core::SystemClock);
    let root = resolve_library(a.library.as_deref())?;
    let lib = Library::open(&root, clock.clone())?;
    let arg = |i: usize| a.rest.get(i).cloned().ok_or_else(|| StoreError::refused("usage", USAGE));
    let open = |q: &str| -> Result<Project, StoreError> { Project::open(&lib.find(q)?, Mode::Write, clock.clone()) };
    match arg(0)?.as_str() {
        "create" => {
            let p = lib.create(&arg(1)?)?;
            let v =
                json!({"id": p.doc().project.id, "name": p.doc().project.name, "bundle": p.bundle(), "rev": p.rev()});
            let text = format!("created {} at {}", p.doc().project.name, p.bundle().display());
            p.close()?;
            Ok((v, text))
        }
        "list" => {
            let rows = lib.list()?;
            let text = rows
                .iter()
                .map(|r| match r {
                    Listing::Ok { entry, bundle } => {
                        format!("{}\t{}\t{} items\t{}", entry.id, entry.name, entry.item_count, bundle.display())
                    }
                    Listing::Damaged { bundle, reason } => format!("DAMAGED\t{}\t{reason}", bundle.display()),
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok((serde_json::to_value(&rows).unwrap(), text))
        }
        "apply" => {
            let src = arg(2)?;
            let text = if src == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s).map_err(|e| StoreError::io("read", "stdin".as_ref(), e))?;
                s
            } else {
                std::fs::read_to_string(&src).map_err(|e| StoreError::io("read", src.as_ref(), e))?
            };
            let ops: Vec<Op> = serde_json::from_str(&text)
                .map_err(|e| StoreError::refused("ops.invalid", format!("the ops file is not a valid op list: {e}")))?;
            let mut p = open(&arg(1)?)?;
            let c = p.apply(ops, a.source, None)?;
            let text = format!("rev {}: {}", c.record.rev, c.record.summary);
            let v = committed_json(&Some(c));
            p.close()?;
            Ok((v, text))
        }
        "history" => {
            let p = Project::open(&lib.find(&arg(1)?)?, Mode::ReadOnly, clock.clone())?;
            let h = p.history()?;
            let text = h
                .iter()
                .map(|e| format!("{}\t{}\t{}", e.rev, e.source.as_str(), e.summary))
                .collect::<Vec<_>>()
                .join("\n");
            Ok((serde_json::to_value(&h).unwrap(), text))
        }
        "undo" | "redo" => {
            let redo = arg(0)? == "redo";
            let mut p = open(&arg(1)?)?;
            let latest = p.latest_record()?.filter(|r| r.rev == p.rev());
            let c = match latest {
                Some(r) if !redo => p.restore_as(r.parent, Source::Undo)?,
                Some(r) if r.source == Source::Undo => p.restore_as(r.parent, Source::Redo)?,
                _ => None,
            };
            let v = committed_json(&c);
            let text = match &c {
                Some(c) => format!("rev {}: {}", c.record.rev, c.record.summary),
                None => format!("nothing to {}", if redo { "redo" } else { "undo" }),
            };
            p.close()?;
            Ok((v, text))
        }
        "restore" => {
            let rev: i64 = arg(2)?.parse().map_err(|_| StoreError::refused("usage", "REV must be a number"))?;
            let mut p = open(&arg(1)?)?;
            let c = p.restore(rev)?;
            let v = committed_json(&c);
            p.close()?;
            Ok((v, format!("restored revision {rev}")))
        }
        "show" => {
            let p = Project::open(&lib.find(&arg(1)?)?, Mode::ReadOnly, clock.clone())?;
            Ok((p.doc().to_json(), p.doc().to_string_pretty()))
        }
        _ => Err(StoreError::refused("usage", USAGE)),
    }
}

fn main() -> ExitCode {
    let a = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", if e.is_empty() { USAGE.to_string() } else { format!("{e}\n{USAGE}") });
            return ExitCode::from(2);
        }
    };
    let json_out = a.json;
    match run(a) {
        Ok((v, text)) => {
            if json_out {
                println!("{v}");
            } else if !text.is_empty() {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json_out {
                eprintln!("{}", json!({"error": {"code": e.code(), "message": e.to_string()}}));
            } else {
                eprintln!("error: {e}");
            }
            if e.code() == "usage" { ExitCode::from(2) } else { ExitCode::from(1) }
        }
    }
}
