//! The running app: `app_context`, `open_in_app`, `seek`, `play`, `pause`.

use serde_json::{Value, json};

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

pub fn context_text(v: &Value) -> String {
    let mut s = String::new();
    match v.get("project").filter(|p| !p.is_null()) {
        Some(p) => s.push_str(&format!(
            "project {} ({}) rev {}\n{}",
            p["name"].as_str().unwrap_or("?"),
            p["id"].as_str().unwrap_or("?"),
            p["rev"],
            p["bundle"].as_str().unwrap_or("")
        )),
        None => s.push_str("no project open"),
    }
    if let Some(pb) = v.get("playback") {
        s.push_str(&format!(
            "\nplayhead {:.3} s ({}){}",
            pb["seconds"].as_f64().unwrap_or(0.0),
            if pb["playing"].as_bool() == Some(true) { "playing" } else { "paused" },
            v.get("viewer").and_then(Value::as_str).map(|m| format!(" · viewer: {m}")).unwrap_or_default()
        ));
    }
    if let Some(sel) = v.get("selection").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        let ids: Vec<&str> = sel.iter().filter_map(Value::as_str).collect();
        s.push_str(&format!("\nselected: {}", ids.join(", ")));
    }
    s
}

pub fn app_context(ctx: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    let v = ctx.app("context", json!({}))?;
    let text = context_text(&v);
    Ok(Receipt::new(v, text))
}

pub fn open_in_app(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    let v = ctx.app("open", json!({"bundle": bundle}))?;
    Ok(Receipt::new(v, format!("opened {} in the app", bundle.display())))
}

pub fn seek(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let seconds = a.f64("seconds").ok_or_else(|| Fail::new("argument.missing", "seek needs `seconds`"))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(Fail::new("argument.invalid", "seconds must be ≥ 0"));
    }
    let v = ctx.app("seek", json!({"seconds": seconds}))?;
    Ok(Receipt::new(v, format!("playhead at {seconds:.3} s")))
}

pub fn play(ctx: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Ok(Receipt::new(ctx.app("play", json!({}))?, "playing"))
}

pub fn pause(ctx: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Ok(Receipt::new(ctx.app("pause", json!({}))?, "paused"))
}
