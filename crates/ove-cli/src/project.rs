//! `ove project …`: list, create, show, rename, duplicate, remove, open in the app, reveal.

use clap::Subcommand;
use ove_core::ops::Op;
use ove_store::Listing;
use serde_json::{Value, json};

use crate::common::{Ctx, Fail, Out, edit, plural, via};

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

const ASPECTS: [&str; 6] = ["16:9", "9:16", "1:1", "4:5", "4:3", "3:4"];

pub fn run(ctx: &Ctx, cmd: &ProjectCmd) -> Out {
    match cmd {
        ProjectCmd::List { search } => {
            let needle = search.as_deref().map(str::to_lowercase);
            let mut rows: Vec<Value> = Vec::new();
            let mut lines = Vec::new();
            let mut listed = ctx.lib.list()?;
            listed.sort_by_key(|l| match l {
                Listing::Ok { entry, .. } => -entry.updated_at,
                Listing::Damaged { .. } => i64::MAX,
            });
            for l in listed {
                match l {
                    Listing::Ok { bundle, entry } => {
                        if needle.as_ref().is_some_and(|n| !entry.name.to_lowercase().contains(n)) {
                            continue;
                        }
                        lines.push(format!(
                            "{}\t{}\t{}\t{:.1}s\t{}",
                            entry.name,
                            entry.id,
                            entry.aspect_ratio,
                            entry.duration,
                            plural(entry.item_count, "item")
                        ));
                        let mut v = serde_json::to_value(&entry).unwrap_or(Value::Null);
                        v["bundle"] = json!(bundle);
                        rows.push(v);
                    }
                    Listing::Damaged { bundle, reason } => {
                        lines.push(format!("(damaged)\t{}\t{reason}", bundle.display()));
                        rows.push(json!({"bundle": bundle, "damaged": reason}));
                    }
                }
            }
            let text = if lines.is_empty() { "no projects".to_string() } else { lines.join("\n") };
            Ok((json!({"projects": rows, "count": rows.len()}), text))
        }
        ProjectCmd::Create { name, aspect, open } => {
            if !ASPECTS.contains(&aspect.as_str()) {
                return Err(Fail::new("argument.invalid", format!("aspect must be one of {}", ASPECTS.join(", "))));
            }
            let name = if name.trim().is_empty() { "Untitled Project" } else { name.trim() };
            let mut p = ctx.lib.create(name)?;
            let mut ops = ove_store::library::starter_tracks();
            if aspect != "16:9" {
                ops.push(Op::SetAspectRatio { aspect_ratio: aspect.clone() });
            }
            p.apply(ops, ctx.source, Some("new project".into()))?;
            p.flush_projection();
            let (id, bundle, rev) = (p.doc().project.id.clone(), p.bundle().to_path_buf(), p.rev());
            p.close()?;
            let mut text = format!("created {name} ({id}) at {}", bundle.display());
            if *open {
                ctx.app("open", json!({"bundle": bundle}))?;
                text.push_str("; opened in the app");
            }
            Ok((
                json!({"id": id, "name": name, "bundle": bundle, "rev": rev, "aspectRatio": aspect, "opened": open}),
                text,
            ))
        }
        ProjectCmd::Show { project } => {
            let p = ctx.read(project)?;
            let d = p.doc();
            let tracks: Vec<Value> = d
                .project
                .tracks
                .iter()
                .map(|t| json!({"id": t.id, "name": t.name, "kind": t.kind, "items": d.items_on_track(&t.id).len()}))
                .collect();
            let pending = d.project.items.values().filter(|i| i.is_pending()).count();
            let v = json!({
                "id": d.project.id,
                "name": d.project.name,
                "bundle": p.bundle(),
                "rev": p.rev(),
                "aspectRatio": d.project.aspect_ratio,
                "durationSeconds": d.duration() as f64 / 1e6,
                "items": d.project.items.len(),
                "pending": pending,
                "media": d.project.media.len(),
                "tracks": tracks,
                "createdAt": d.project.created_at,
                "updatedAt": d.project.updated_at,
            });
            let mut text = format!(
                "{} ({})\n{} · {:.2} s · {} · {} · rev {}\n{}",
                d.project.name,
                d.project.id,
                d.project.aspect_ratio,
                d.duration() as f64 / 1e6,
                plural(d.project.items.len(), "item"),
                plural(d.project.media.len(), "media file"),
                p.rev(),
                p.bundle().display()
            );
            for t in &d.project.tracks {
                text.push_str(&format!(
                    "\n  {}\t{}\t{}\t{}",
                    t.id,
                    t.name,
                    t.kind,
                    plural(d.items_on_track(&t.id).len(), "item")
                ));
            }
            if pending > 0 {
                text.push_str(&format!("\n{} pending", plural(pending, "placeholder")));
            }
            Ok((v, text))
        }
        ProjectCmd::Rename { project, name } => {
            if name.trim().is_empty() {
                return Err(Fail::new("argument.invalid", "the name is empty"));
            }
            let (rev, how) = edit(ctx, project, |w| {
                let mut set = serde_json::Map::new();
                set.insert("name".into(), name.trim().into());
                let rev = w.apply(
                    vec![Op::UpdateProject { set, unset: vec![] }],
                    ctx.source,
                    format!("rename to {}", name.trim()),
                )?;
                Ok((rev, via(w)))
            })?;
            Ok((json!({"rev": rev, "name": name.trim()}), format!("rev {rev}: renamed to {}{how}", name.trim())))
        }
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
        ProjectCmd::Open { project } => {
            let bundle = ctx.find(project)?;
            let v = ctx.app("open", json!({"bundle": bundle}))?;
            Ok((v, format!("opened {} in the app", bundle.display())))
        }
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
