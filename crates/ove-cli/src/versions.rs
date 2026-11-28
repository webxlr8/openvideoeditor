//! `ove version …`: save, list, print, diff and restore named versions (registry tools `save_version`,
//! `list_versions`, `print_version`, `diff_versions`, `restore_version`).

use clap::Subcommand;
use serde_json::json;

use crate::common::{Ctx, Out};

#[derive(Subcommand)]
pub enum VersionCmd {
    /// Save the current revision as a named version.
    Save {
        project: String,
        name: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// List named versions, oldest first.
    #[command(alias = "ls")]
    List { project: String },
    /// Print a version's timeline projection.
    Print {
        project: String,
        name: String,
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        lines: Option<i64>,
    },
    /// Compare two versions, or a version and the current timeline: added, removed, moved, trimmed, changed.
    Diff { project: String, from: String, to: Option<String> },
    /// Make the timeline equal to a version again (one undoable revision).
    Restore { project: String, name: String },
}

pub fn run(ctx: &Ctx, cmd: &VersionCmd) -> Out {
    match cmd {
        VersionCmd::Save { project, name, note } => {
            ctx.tool("save_version", json!({"project": project, "name": name, "note": note}))
        }
        VersionCmd::List { project } => ctx.tool("list_versions", json!({"project": project})),
        VersionCmd::Print { project, name, from, lines } => {
            ctx.tool("print_version", json!({"project": project, "name": name, "from": from, "lines": lines}))
        }
        VersionCmd::Diff { project, from, to } => {
            ctx.tool("diff_versions", json!({"project": project, "from": from, "to": to}))
        }
        VersionCmd::Restore { project, name } => ctx.tool("restore_version", json!({"project": project, "name": name})),
    }
}
