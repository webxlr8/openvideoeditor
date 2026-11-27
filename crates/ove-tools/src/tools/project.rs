//! `list_projects`, `create_project`, `show_project`, `rename_project`.

use ove_core::ops::Op;
use ove_store::Listing;
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, edit, read, via};
use crate::fail::{Fail, plural};
use crate::receipt::Receipt;
use crate::spec::Args;

pub const ASPECTS: [&str; 6] = ["16:9", "9:16", "1:1", "4:5", "4:3", "3:4"];

pub fn list_projects(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let needle = a.str("search").map(str::to_lowercase);
    let mut rows: Vec<Value> = Vec::new();
    let mut lines = Vec::new();
    let mut listed = ctx.library()?.list()?;
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
    Ok(Receipt::new(json!({"projects": rows, "count": rows.len()}), text))
}

pub fn create_project(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let aspect = a.str("aspect").unwrap_or("16:9").to_string();
    if !ASPECTS.contains(&aspect.as_str()) {
        return Err(Fail::new("argument.invalid", format!("aspect must be one of {}", ASPECTS.join(", "))));
    }
    let name = a.req_str("name")?;
    let name = if name.trim().is_empty() { "Untitled Project" } else { name.trim() };
    let source = ctx.source();
    let mut p = ctx.library()?.create(name)?;
    let mut ops = ove_store::library::starter_tracks();
    if aspect != "16:9" {
        ops.push(Op::SetAspectRatio { aspect_ratio: aspect.clone() });
    }
    p.apply(ops, source, Some("new project".into()))?;
    p.flush_projection();
    let (id, bundle, rev) = (p.doc().project.id.clone(), p.bundle().to_path_buf(), p.rev());
    p.close()?;
    let mut text = format!("created {name} ({id}) at {}", bundle.display());
    let open = a.bool("open");
    if open {
        ctx.app("open", json!({"bundle": bundle}))?;
        text.push_str("; opened in the app");
    }
    Ok(Receipt::new(
        json!({"id": id, "name": name, "bundle": bundle, "rev": rev, "aspectRatio": aspect, "opened": open}),
        text,
    ))
}

pub fn show_project(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
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
        Ok(Receipt::new(v, text))
    })
}

/// `use_project`: the session's default project (refused in a session pinned to another project).
pub fn use_project(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(Some(a.req_str("project")?))?;
    ctx.use_project(&bundle)?;
    read(ctx, &bundle, |p| {
        let d = &p.doc().project;
        Ok(Receipt::new(
            json!({"id": d.id, "name": d.name, "bundle": p.bundle(), "rev": p.rev()}),
            format!("calls without `project` now use {} ({})", d.name, d.id),
        ))
    })
}

pub fn rename_project(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let name = a.req_str("name")?.trim().to_string();
    if name.is_empty() {
        return Err(Fail::new("argument.invalid", "the name is empty"));
    }
    let bundle = ctx.bundle(a.project())?;
    let (rev, how) = edit(ctx, &bundle, |w| {
        let mut set = serde_json::Map::new();
        set.insert("name".into(), name.clone().into());
        let rev = w.apply(vec![Op::UpdateProject { set, unset: vec![] }], format!("rename to {name}"))?;
        Ok((rev, via(w)))
    })?;
    Ok(Receipt::new(json!({"rev": rev, "name": name}), format!("rev {rev}: renamed to {name}{how}")))
}
