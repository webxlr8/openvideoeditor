//! The timeline as text: `print_timeline`, `check_timeline`, `apply_timeline`, `verify_timeline`,
//! `timeline_history`, `undo_timeline`.

use std::collections::BTreeSet;

use ove_projection::textedit::{TextEdit, apply_edits};
use ove_projection::verify::{Context, Level, Registry, has_errors};
use ove_projection::{ApplyInput, BaseError, Plan, Prefer, Refusal};
use ove_store::media::{MediaState, media_status};
use ove_store::project::Project;
use serde_json::{Value, json};

use crate::ctx::{Surface, ToolCtx, edit, read, via};
use crate::fail::{Fail, now_ms};
use crate::receipt::Receipt;
use crate::spec::Args;

/// The printed projection, or a line range of it.
pub fn print_receipt(p: &Project, from: Option<usize>, lines: Option<usize>) -> Receipt {
    let full = p.projection();
    let text = ove_projection::print::slice_lines(&full, from.unwrap_or(1), lines);
    let mut v = json!({"rev": p.rev(), "text": text});
    if from.is_some() || lines.is_some() {
        let start = from.unwrap_or(1);
        v["range"] = json!({
            "from": start,
            "to": (start + text.lines().count()).saturating_sub(1),
            "total": full.lines().count(),
        });
    }
    let shown = text.trim_end_matches('\n').to_string();
    Receipt::new(v, shown)
}

/// `print_timeline` for a model: the text itself, with the range when it is part of the timeline.
pub fn print_model(r: &Receipt) -> String {
    let text = r.json["text"].as_str().unwrap_or_default();
    match r.json.get("range") {
        Some(g) => format!("{text}\n[lines {}–{} of {}]", g["from"], g["to"], g["total"]),
        None => text.to_string(),
    }
}

pub fn print_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let positive = |k: &str| -> Result<Option<usize>, Fail> {
        match a.i64(k) {
            None => Ok(None),
            Some(n) if n >= 1 => Ok(Some(n as usize)),
            Some(_) => Err(Fail::new("argument.invalid", format!("`{k}` must be a positive integer"))),
        }
    };
    let (from, lines) = (positive("from")?, positive("lines")?);
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| Ok(print_receipt(p, from, lines)))
}

/// Plans applying a projection text onto the project (3-way merge with the text's base revision).
pub fn plan_for(p: &Project, text: &str, prefer: Prefer) -> Result<Plan, Refusal> {
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
        now_ms: now_ms(),
    })
}

pub fn plan_json(p: &Plan) -> Value {
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

pub fn plan_text(p: &Plan) -> String {
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

fn prefer(a: &Args) -> Result<Prefer, Fail> {
    match a.str("prefer") {
        None | Some("file") => Ok(Prefer::File),
        Some("editor") => Ok(Prefer::Editor),
        Some(other) => {
            Err(Fail::new("argument.invalid", format!("`prefer` must be \"file\" or \"editor\", not {other}")))
        }
    }
}

pub fn check_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let text = a.req_str("source")?.to_string();
    let prefer = prefer(a)?;
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let plan = plan_for(p, &text, prefer)?;
        let mut v = plan_json(&plan);
        v["ok"] = json!(true);
        Ok(Receipt::new(v, format!("ok: {}", plan_text(&plan))))
    })
}

/// The text edits of an `apply_timeline` call.
fn edits(a: &Args) -> Result<Option<Vec<TextEdit>>, Fail> {
    match a.value("edits") {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| Fail::new("argument.invalid", format!("`edits` must be a list of {{oldText, newText}}: {e}"))),
    }
}

/// Applies text edits to the printed projection of `p` (which must be at `rev` when given).
fn edited_text(p: &Project, rev: Option<i64>, edits: &[TextEdit]) -> Result<String, Fail> {
    if let Some(r) = rev
        && r != p.rev()
    {
        return Err(Fail::new(
            "rev.stale",
            format!(
                "The timeline changed since rev {r} (it is now rev {}). Print it again and redo your edits against the new text.",
                p.rev()
            ),
        ));
    }
    apply_edits(&p.projection(), edits).map_err(|e| {
        let code = if e.code == "edit.notFound" { "edit.anchor" } else { e.code };
        Fail::new(code, format!("{}. Print the timeline again if it changed.", e.message))
            .details(json!({"index": e.index, "positions": e.positions}))
    })
}

pub fn apply_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let edits = edits(a)?;
    let source = a.str("source").map(str::to_string);
    if edits.is_some() == source.is_some() {
        return Err(Fail::new("argument.invalid", "give exactly one of `edits` or `source`"));
    }
    let prefer = prefer(a)?;
    let rev = a.i64("rev");
    let label = match ctx.surface() {
        Surface::InApp => "assistant",
        _ => "source apply",
    };
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let text = match (&edits, &source) {
            (Some(e), _) => edited_text(w.project(), rev, e)?,
            (None, Some(s)) => s.clone(),
            (None, None) => return Err(Fail::new("argument.invalid", "give `edits` or `source`")),
        };
        let plan = plan_for(w.project(), &text, prefer)?;
        let mut v = plan_json(&plan);
        let changed = !plan.ops.is_empty();
        if changed {
            w.apply(plan.ops.clone(), format!("{label}: {}", plan.summary()))?;
        }
        w.set_diagnostics(plan.warnings.iter().map(|w| w.to_string()).collect())?;
        v["rev"] = json!(w.rev());
        v["changed"] = json!(changed);
        v["viaApp"] = json!(w.via_app());
        v["summary"] = json!(plan.summary());
        let text = if changed {
            format!("rev {}: {}{}", w.rev(), plan_text(&plan), via(w))
        } else {
            format!("no changes (rev {})", w.rev())
        };
        Ok(Receipt::new(v, text))
    })
}

/// Lint findings for the project, as a receipt; `Err(verify.failed)` when any is an error.
pub fn verify_receipt(p: &Project) -> Result<Receipt, Fail> {
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
                        Level::Error => "error",
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
    Ok(Receipt::new(v, text))
}

pub fn verify_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, verify_receipt)
}

pub fn timeline_history(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let h = p.history()?;
        let text =
            h.iter().map(|e| format!("{}\t{}\t{}", e.rev, e.source.as_str(), e.summary)).collect::<Vec<_>>().join("\n");
        Ok(Receipt::new(json!({"rev": p.rev(), "revisions": h}), text))
    })
}

/// `undo_timeline` for a model.
pub fn undo_model(r: &Receipt) -> String {
    match (r.json["undone"].as_str(), r.json["rev"].as_i64(), r.json["changed"].as_bool()) {
        (Some(s), Some(rev), Some(true)) => format!("Undid \"{s}\" (now rev {rev})."),
        (_, Some(rev), Some(true)) => format!("Returned to an earlier state (now rev {rev})."),
        _ => "Nothing changed.".into(),
    }
}

pub fn undo_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let rev = a.i64("rev");
    let agent = ctx.surface().is_agent();
    let own = ctx.source();
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let (target, undone) = match rev {
            Some(r) => (r, None),
            None => match w.project().latest_record()?.filter(|r| r.rev == w.rev()) {
                Some(r) if agent && r.source != own => {
                    return Err(Fail::new(
                        "undo.notYours",
                        format!("The latest change (\"{}\") was not made by you; not undoing it.", r.summary),
                    ));
                }
                Some(r) => (r.parent, Some(r.summary)),
                None => return Err(Fail::new("undo.nothing", "There is nothing to undo.")),
            },
        };
        let c = w.restore(target)?;
        let v =
            json!({"rev": w.rev(), "changed": c.is_some(), "target": target, "viaApp": w.via_app(), "undone": undone});
        let text = match c {
            Some((r, summary)) => format!("rev {r}: {summary}{}", via(w)),
            None => format!("already at the state of revision {target}"),
        };
        Ok(Receipt::new(v, text))
    })
}
