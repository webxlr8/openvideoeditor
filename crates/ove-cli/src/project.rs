//! `ove project …`: list, create, show, rename, duplicate, remove, open in the app, reveal.

use clap::Subcommand;
use serde_json::json;

use crate::common::{Ctx, Fail, Out};

#[derive(Subcommand)]
pub enum ProjectCmd {
    /// List the projects in the library, most recently changed first.
    #[command(alias = "ls")]
    List {
        /// Only projects whose name contains this text (case-insensitive).
        #[arg(long)]
        search: Option<String>,
    },
    /// Create a project with the starter tracks (Text, Captions, Overlay, Main, Audio).
    Create {
        name: String,
        /// 16:9, 9:16, 1:1, 4:5, 4:3 or 3:4.
        #[arg(long, default_value = "16:9")]
        aspect: String,
        /// Also open it in the running app.
        #[arg(long)]
        open: bool,
    },
    /// Name, id, aspect ratio, duration, tracks, media and revision.
    Show { project: String },
    /// Rename (the bundle folder keeps its name).
    Rename { project: String, name: String },
    /// Copy a project (new id; media owned by the bundle is copied, linked media stays linked).
    Duplicate {
        project: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Move a project to the Trash. Refused while it is open anywhere.
    #[command(alias = "rm")]
    Remove {
        project: String,
        /// Required: confirms the removal.
        #[arg(short, long)]
        yes: bool,
    },
    /// Open the project in the running app.
    Open { project: String },
    /// Show the project bundle in the file manager.
    Reveal { project: String },
}

pub fn run(ctx: &Ctx, cmd: &ProjectCmd) -> Out {
    match cmd {
        ProjectCmd::List { search } => ctx.tool("list_projects", json!({"search": search})),
        ProjectCmd::Create { name, aspect, open } => {
            ctx.tool("create_project", json!({"name": name, "aspect": aspect, "open": open}))
        }
        ProjectCmd::Show { project } => ctx.tool("show_project", json!({"project": project})),
        ProjectCmd::Rename { project, name } => ctx.tool("rename_project", json!({"project": project, "name": name})),
        ProjectCmd::Duplicate { project, name } => {
            let dest = ctx.lib.duplicate(&ctx.find(project)?, name.as_deref())?;
            let p = ctx.read(&dest.display().to_string())?;
            let v = json!({"id": p.doc().project.id, "name": p.doc().project.name, "bundle": dest});
            Ok((v, format!("duplicated as {} at {}", p.doc().project.name, dest.display())))
        }
        ProjectCmd::Remove { project, yes } => {
            let bundle = ctx.find(project)?;
            if !yes {
                return Err(Fail::new(
                    "confirm.required",
                    format!("this moves {} to the Trash; run again with --yes", bundle.display()),
                ));
            }
            ctx.lib.trash(&bundle)?;
            Ok((json!({"bundle": bundle, "removed": true}), format!("moved {} to the Trash", bundle.display())))
        }
        ProjectCmd::Open { project } => ctx.tool("open_in_app", json!({"project": project})),
        ProjectCmd::Reveal { project } => {
            let bundle = ctx.find(project)?;
            reveal(&bundle)?;
            Ok((json!({"bundle": bundle}), bundle.display().to_string()))
        }
    }
}

/// Shows a path in the system file manager.
pub fn reveal(p: &std::path::Path) -> Result<(), Fail> {
    let r = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(p).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn()
    };
    r.map(|_| ()).map_err(|e| Fail::new("reveal.failed", format!("could not open the file manager: {e}")))
}
