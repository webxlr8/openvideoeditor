//! `edit_timeline`: one edit verb of the shared planners (split, trim, ripple trim, ripple delete, delete, insert,
//! overwrite, lift, extract, close gap, slip, slide, roll, move) as one revision. A request made against an older
//! revision (`rev`) is moved through the ripples made since, so it lands where it was meant.

use ove_edit::Request;
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, edit, via};
use crate::fail::{Fail, plural};
use crate::receipt::Receipt;
use crate::spec::Args;

fn refused(r: ove_edit::Refusal) -> Fail {
    Fail::new(r.code.clone(), r.message.clone()).details(json!({"culprits": r.culprits}))
}

/// The request: a JSON object, or JSON text of one (what the command line and older clients send).
fn request(a: &Args) -> Result<Request, Fail> {
    let v = a.value("request").ok_or_else(|| Fail::new("argument.missing", "edit_timeline needs `request`"))?;
    let v = match v {
        Value::String(s) => serde_json::from_str::<Value>(s)
            .map_err(|e| Fail::new("argument.invalid", format!("`request` is not valid JSON: {e}")))?,
        other => other.clone(),
    };
    serde_json::from_value(v)
        .map_err(|e| Fail::new("argument.invalid", format!("`request` is not a valid edit request: {e}")))
}

/// Item ids for new items, unique per process.
fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("i_{:x}{}", crate::fail::now_ms(), N.fetch_add(1, Ordering::Relaxed))
}

pub fn edit_timeline(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let req = request(a)?;
    let (rev, preview) = (a.i64("rev"), a.bool("preview"));
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let current = w.rev();
        let base = rev.unwrap_or(current);
        if base > current {
            return Err(Fail::new(
                "rev.newer",
                format!("revision {base} is newer than the project's current revision {current}"),
            ));
        }
        let ripples = if base < current { w.project().ripples_since(base) } else { vec![] };
        let req = req.rebased(w.doc(), &ripples);
        let plan = ove_edit::plan_with(w.doc(), &req, &mut new_id).map_err(refused)?;
        let mut v = json!({
            "verb": req.verb(),
            "request": req,
            "baseRev": base,
            "summary": plan.summary,
            "touched": plan.touched,
            "limited": plan.limited,
            "ops": plan.ops,
            "timeMap": plan.time_map,
            "dryRun": preview,
        });
        if preview || plan.ops.is_empty() {
            v["rev"] = current.into();
            let what = if plan.ops.is_empty() { "no change" } else { "would apply" };
            return Ok(Receipt::new(v, format!("{what}: {} ({})", plan.summary, plural(plan.touched.len(), "item"))));
        }
        let before = w.doc().clone();
        let new = w.apply_mapped(plan.ops.clone(), plan.summary.clone(), plan.time_map.clone())?;
        v["rev"] = new.into();
        // The receipt's printed lines, as apply_timeline gives them, for the agent's next text edit.
        crate::edits::remember_doc(w.bundle(), new, w.doc());
        if let (Value::Object(m), Value::Object(r)) = (&mut v, crate::edits::receipt(&before, w.doc())) {
            m.extend(r);
        }
        let text = format!("rev {new}: {}{}", plan.summary, via(w));
        Ok(Receipt::new(v, text))
    })
}
