//! The timeline as text: `print_timeline`, `check_timeline`, `apply_timeline`, `verify_timeline`,
//! `timeline_history`, `undo_timeline`.

use std::collections::BTreeSet;

use ove_core::revision::{Actor, ActorKind, RevisionRecord, Source};
use ove_projection::textedit::TextEdit;
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
    crate::edits::remember(p.bundle(), p.rev(), &full);
    crate::edits::remember_doc(p.bundle(), p.rev(), p.doc());
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
        crate::edits::doc_at(p, rev).map_err(|e| BaseError {
            code: match e.code() {
                "revision.missing" => "rev.pruned".into(),
                "revision.newer" => "rev.newer".into(),
                c => c.to_string(),
            },
            message: format!("the projection's base revision {rev} is not available ({e})"),
        })
    };
    // Rebased through ripples made since the text's revision, so a concurrent ripple edit still lines up.
    ove_projection::plan_rebased(
        &ApplyInput { text, current: p.doc(), base_for: &base_for, bundle: Some(p.bundle()), prefer, now_ms: now_ms() },
        &|rev| p.ripples_since(rev),
    )
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
            (Some(e), _) => crate::edits::edited_text(w.project(), rev, e)?,
            (None, Some(s)) => s.clone(),
            (None, None) => return Err(Fail::new("argument.invalid", "give `edits` or `source`")),
        };
        let plan = plan_for(w.project(), &text, prefer)?;
        let mut v = plan_json(&plan);
        let changed = !plan.ops.is_empty();
        let before = w.doc().clone();
        if changed {
            w.apply(plan.ops.clone(), format!("{label}: {}", plan.summary()))?;
        }
        w.set_diagnostics(plan.warnings.iter().map(|w| w.to_string()).collect())?;
        // The receipt: the printed lines of what changed, for the next edit's anchors.
        crate::edits::remember_doc(w.bundle(), w.rev(), w.doc());
        if let (Value::Object(m), Value::Object(r)) = (&mut v, crate::edits::receipt(&before, w.doc())) {
            for (k, x) in r {
                if k != "removedItems" {
                    m.insert(k, x);
                }
            }
        }
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
        let text = h
            .iter()
            .map(|e| {
                let by = match &e.actor {
                    Some(a) => match &a.client {
                        Some(c) => format!("\t{} ({c})", a.kind.as_str()),
                        None => format!("\t{}", a.kind.as_str()),
                    },
                    None => String::new(),
                };
                format!("{}\t{}\t{}{by}", e.rev, e.source.as_str(), e.summary)
            })
            .collect::<Vec<_>>()
            .join("\n");
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

/// The caller's own revisions (not undos), oldest first: those of the same session — and of `turn` when given —
/// or, when the caller has no session, those with the caller's source.
fn own_revisions(
    p: &Project,
    actor: Option<&Actor>,
    source: Source,
    turn: Option<&str>,
) -> Result<Vec<RevisionRecord>, Fail> {
    // Scoped selection lives in the store (add-markers-keyframe-lanes-and-versions D9).
    use ove_store::undo::{UndoScope, in_scope};
    let scope = match (actor, turn) {
        (Some(a), Some(t)) if a.session.is_some() => Some(UndoScope::Turn(a.clone(), t.to_string())),
        (Some(a), None) if a.session.is_some() => Some(UndoScope::Session(a.clone())),
        _ => None,
    };
    Ok(p.records()?
        .into_iter()
        .filter(|r| match &scope {
            Some(s) => in_scope(r, s),
            None => r.source != Source::Undo && r.source == source,
        })
        .collect())
}

/// True when reverting `r` would change nothing (it was undone already, or its change is gone).
fn already_undone(p: &Project, r: &RevisionRecord) -> bool {
    match ove_core::apply_batch(p.doc(), &r.inverse) {
        Ok(a) => {
            a.doc.project.items == p.doc().project.items
                && a.doc.project.tracks == p.doc().project.tracks
                && a.doc.project.media == p.doc().project.media
                && a.doc.project.name == p.doc().project.name
                && a.doc.project.aspect_ratio == p.doc().project.aspect_ratio
        }
        Err(_) => true,
    }
}

pub fn undo_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let rev = a.i64("rev");
    if !ctx.surface().is_agent() {
        return restore_latest_or(ctx, a, rev);
    }
    // Agents undo only their own revisions, one at a time, newest first.
    let (actor, source) = (ctx.actor(), ctx.source());
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let own = own_revisions(w.project(), actor.as_ref(), source, None)?;
        let target = match rev {
            Some(r) => own.iter().find(|x| x.rev == r).ok_or_else(|| {
                Fail::new(
                    "undo.notYours",
                    format!("Revision {r} was not made by you; agents can undo only their own changes."),
                )
            })?,
            None => own.iter().rev().find(|r| !already_undone(w.project(), r)).ok_or_else(|| {
                Fail::new(
                    "undo.nothing",
                    "There is nothing of yours to undo (changes made by others are never undone).",
                )
            })?,
        };
        let new = w.undo_selected(&[target.rev], format!("undo: {}", target.summary))?;
        let v = json!({"rev": new, "changed": true, "undoneRevs": [target.rev], "undone": target.summary, "viaApp": w.via_app()});
        Ok(Receipt::new(v, format!("rev {new}: undid \"{}\"{}", target.summary, via(w))))
    })
}

/// The command line's `ove undo`: the latest revision, whoever made it, or back to the state of `rev`.
fn restore_latest_or(ctx: &mut dyn ToolCtx, a: &Args, rev: Option<i64>) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let (target, undone) = match rev {
            Some(r) => (r, None),
            None => match w.project().latest_record()?.filter(|r| r.rev == w.rev()) {
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

/// `undo_my_changes`: every revision of the caller's session, or of one turn, as one new revision.
pub fn undo_my_changes(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let scope = a.str("scope").unwrap_or("session");
    let cli = !ctx.surface().is_agent();
    let mut actor = ctx.actor();
    let turn = match scope {
        "session" | "mine" => None,
        "turn" => {
            Some(a.str("turn").map(str::to_string).or_else(|| actor.as_ref().and_then(|x| x.turn.clone())).ok_or_else(
                || Fail::new("argument.missing", "scope `turn` needs `turn` (this session has no current turn)"),
            )?)
        }
        "any" if cli => {
            // The command line may undo any session's changes, named by `session` (and optionally `turn`).
            let session = a.req_str("session")?;
            let kind = match a.str("kind") {
                Some(k) => serde_json::from_value(json!(k))
                    .map_err(|_| Fail::new("argument.invalid", format!("unknown actor kind {k}")))?,
                None => ActorKind::Mcp,
            };
            actor = Some(Actor { kind, session: Some(session.to_string()), turn: None, client: None });
            a.str("turn").map(str::to_string)
        }
        "any" => {
            return Err(Fail::new(
                "undo.scope",
                "agents can undo only their own changes; scope `any` is for the command line",
            ));
        }
        other => {
            return Err(Fail::new("argument.invalid", format!("`scope` must be session, turn or any, not {other}")));
        }
    };
    if actor.as_ref().is_none_or(|x| x.session.is_none()) {
        return Err(Fail::new("undo.noSession", "this caller has no session, so it has no changes of its own to undo"));
    }
    let source = ctx.source();
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let own: Vec<RevisionRecord> = own_revisions(w.project(), actor.as_ref(), source, turn.as_deref())?
            .into_iter()
            .filter(|r| !already_undone(w.project(), r))
            .collect();
        if own.is_empty() {
            return Err(Fail::new("undo.nothing", "There is nothing of yours to undo in that scope."));
        }
        let revs: Vec<i64> = own.iter().map(|r| r.rev).collect();
        let what = match &turn {
            Some(t) => format!("turn {t}"),
            None => "session".to_string(),
        };
        let new = w.undo_selected(
            &revs,
            format!("undo the {what} ({} change{})", revs.len(), if revs.len() == 1 { "" } else { "s" }),
        )?;
        let v = json!({"rev": new, "changed": true, "undoneRevs": revs, "viaApp": w.via_app()});
        Ok(Receipt::new(
            v,
            format!("rev {new}: undid {} change{}{}", revs.len(), if revs.len() == 1 { "" } else { "s" }, via(w)),
        ))
    })
}
