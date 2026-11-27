//! Named versions (add-markers-keyframe-lanes-and-versions task 4.3): `save_version`, `list_versions`,
//! `print_version`, `diff_versions` and `restore_version`. A version pins a revision with a snapshot that history
//! pruning never removes; restoring is one new revision, undoable like any other.

use serde_json::{Value, json};

use ove_store::versions::Version;

use crate::ctx::{ToolCtx, edit, read, via};
use crate::fail::{Fail, plural};
use crate::receipt::Receipt;
use crate::spec::Kind::{Int, Str};
use crate::spec::{Arg, Args, arg, req};

pub const SAVE_VERSION_ABOUT: &str = "Save the current revision as a named version (\"client v2\") with an optional note. Versions survive history pruning and can be printed, compared and restored later. A name already in use is refused with `version.exists`.";
pub const SAVE_VERSION_ARGS: &[Arg] =
    &[crate::registry::PROJECT, req("name", Str, "Version name"), arg("note", Str, "What this version is")];
pub const LIST_VERSIONS_ABOUT: &str =
    "List the project's named versions, oldest first: name, revision, time, who saved it, note and the bytes it added.";
pub const LIST_VERSIONS_ARGS: &[Arg] = &[crate::registry::PROJECT];
pub const PRINT_VERSION_ABOUT: &str = "Print a named version's timeline as its TSX projection (read-only: apply edits to the current timeline, not to a version).";
pub const PRINT_VERSION_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    req("name", Str, "Version name"),
    arg("from", Int, "First line (1-based)"),
    arg("lines", Int, "Number of lines"),
];
pub const DIFF_VERSIONS_ABOUT: &str = "Compare two named versions, or a version and the current timeline (omit `to`): the items and markers added, removed, moved, trimmed or otherwise changed, with their old and new times.";
pub const DIFF_VERSIONS_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    req("from", Str, "The older version's name"),
    arg("to", Str, "The newer version's name (default: the current timeline)"),
];
pub const RESTORE_VERSION_ABOUT: &str =
    "Make the timeline equal to a named version again, as one new revision (one undo step brings back what was there).";
pub const RESTORE_VERSION_ARGS: &[Arg] = &[crate::registry::PROJECT, req("name", Str, "Version name")];

fn version_json(v: &Version) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn line(v: &Version) -> String {
    let note = v.note.as_ref().map(|n| format!(" — {n}")).unwrap_or_default();
    format!("{}\trev {}\t{} bytes{note}", v.name, v.rev, v.bytes)
}

pub fn save_version(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let name = a.req_str("name")?.to_string();
    let note = a.str("note").map(str::to_string);
    let actor = ctx.actor();
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let v = p.save_version(&name, note.as_deref(), actor.clone(), crate::fail::now_ms())?;
        Ok(Receipt::new(json!({"version": version_json(&v)}), format!("saved version {}", line(&v))))
    })
}

pub fn list_versions(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let all = p.versions()?;
        let mut text = plural(all.len(), "version");
        for v in &all {
            text.push('\n');
            text.push_str(&line(v));
        }
        Ok(Receipt::new(json!({"versions": all.iter().map(version_json).collect::<Vec<_>>()}), text))
    })
}

pub fn print_version(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let name = a.req_str("name")?.to_string();
    let (from, lines) = (a.i64("from"), a.i64("lines"));
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let d = p.version_doc(&name)?;
        let full = ove_projection::print(&d, &Default::default());
        let text = match from {
            Some(f) => ove_projection::print::slice_lines(&full, f.max(1) as usize, lines.map(|n| n.max(0) as usize)),
            None => full,
        };
        Ok(Receipt::new(json!({"name": name, "rev": d.project.rev, "text": text}), text))
    })
}

pub fn diff_versions(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let from = a.req_str("from")?.to_string();
    let to = a.str("to").map(str::to_string);
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let changes = p.version_diff(&from, to.as_deref())?;
        let target = to.clone().unwrap_or_else(|| "the current timeline".into());
        let mut text = format!(
            "{} from \"{from}\" to {}",
            plural(changes.len(), "change"),
            if to.is_some() { format!("\"{target}\"") } else { target.clone() }
        );
        for c in &changes {
            text.push('\n');
            text.push_str(&c.line());
        }
        Ok(Receipt::new(json!({"from": from, "to": to, "changes": changes}), text))
    })
}

pub fn restore_version(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let name = a.req_str("name")?.to_string();
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let target = w.project().version_doc(&name)?;
        let ops = ove_core::diff::diff_documents(w.doc(), &target);
        if ops.is_empty() {
            return Ok(Receipt::new(
                json!({"rev": w.rev(), "changed": false}),
                format!("already equal to version \"{name}\""),
            ));
        }
        let rev = w.apply(ops, format!("restore version \"{name}\""))?;
        Ok(Receipt::new(
            json!({"rev": rev, "changed": true}),
            format!("rev {rev}: restored version \"{name}\"{}", via(w)),
        ))
    })
}
