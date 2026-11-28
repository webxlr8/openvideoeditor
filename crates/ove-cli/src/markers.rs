//! `ove markers …`: list, add, update and remove timeline markers (registry tools `list_markers`, `add_marker`,
//! `update_marker`, `remove_markers`). People use the command line, so it may resolve markers.

use clap::Subcommand;
use serde_json::json;

use crate::common::{Ctx, Out};

#[derive(Subcommand)]
pub enum MarkersCmd {
    /// List markers in time order, with the items under each one.
    #[command(alias = "ls")]
    List {
        project: String,
        /// Only markers with this status: open, review or resolved.
        #[arg(long)]
        status: Option<String>,
        /// Only markers assigned to this name.
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Add a marker at a time in seconds (a range with --dur).
    Add {
        project: String,
        /// Timeline time in seconds.
        #[arg(long)]
        at: f64,
        /// Duration in seconds (default 0: a point marker).
        #[arg(long)]
        dur: Option<f64>,
        #[arg(long)]
        name: Option<String>,
        /// red, orange, yellow, green, teal, blue, purple, pink or #rrggbb.
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        note: Option<String>,
        /// open (default), review or resolved.
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Change a marker.
    Update {
        project: String,
        id: String,
        #[arg(long)]
        at: Option<f64>,
        #[arg(long)]
        dur: Option<f64>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        /// Fields to remove (name, color, note, assignee), comma-separated.
        #[arg(long, value_delimiter = ',')]
        unset: Vec<String>,
    },
    /// Remove markers by id.
    #[command(alias = "rm")]
    Remove {
        project: String,
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

pub fn run(ctx: &Ctx, cmd: &MarkersCmd) -> Out {
    match cmd {
        MarkersCmd::List { project, status, assignee } => {
            ctx.tool("list_markers", json!({"project": project, "status": status, "assignee": assignee}))
        }
        MarkersCmd::Add { project, at, dur, name, color, note, status, assignee } => ctx.tool(
            "add_marker",
            json!({"project": project, "at": at, "dur": dur, "name": name, "color": color, "note": note,
                   "status": status, "assignee": assignee}),
        ),
        MarkersCmd::Update { project, id, at, dur, name, color, note, status, assignee, unset } => ctx.tool(
            "update_marker",
            json!({"project": project, "id": id, "at": at, "dur": dur, "name": name, "color": color, "note": note,
                   "status": status, "assignee": assignee, "unset": unset}),
        ),
        MarkersCmd::Remove { project, ids } => ctx.tool("remove_markers", json!({"project": project, "ids": ids})),
    }
}
