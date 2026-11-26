//! `check_code` (motion-graphics code items) and `ask_user` (answered by the app's question card).

use serde_json::{Value, json};

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

/// Frame size for a project aspect ratio.
fn size_for(aspect: Option<&str>) -> (u32, u32) {
    match aspect {
        Some("9:16") => (1080, 1920),
        Some("1:1") => (1080, 1080),
        Some("4:5") => (1080, 1350),
        Some("4:3") => (1440, 1080),
        Some("3:4") => (1080, 1440),
        _ => (1920, 1080),
    }
}

/// The motion-graphics check as JSON. With a project, the frame size follows its aspect ratio and image assets
/// (media ids or names) load from its media.
#[cfg(feature = "graphics")]
pub fn check(p: Option<&ove_store::project::Project>, composition: &Value, duration: f64) -> Value {
    let Some(c) = ove_graphics::CodeComposition::from_value(composition) else {
        return json!({"ok": false, "diagnostics": ["error [shape] the composition must be an object"]});
    };
    let size = size_for(p.map(|p| p.doc().project.aspect_ratio.as_str()));
    let load = |id: &str| {
        let p = p?;
        let doc = p.doc();
        let id = if doc.registry.contains_key(id) {
            id.to_string()
        } else {
            ove_projection::print::media_names(doc).into_iter().find(|(_, n)| n == id).map(|(k, _)| k)?
        };
        let native = std::path::PathBuf::from(&doc.registry.get(&id)?.native_path);
        let path = if native.is_absolute() { native } else { p.bundle().join(native) };
        std::fs::read(path).ok().map(std::sync::Arc::new)
    };
    let r = ove_graphics::check(&c, size, duration, &load);
    let fmt = |d: &ove_graphics::Diagnostic| {
        let at = match (d.line, d.column) {
            (Some(l), Some(c)) => format!(" {l}:{c}"),
            _ => String::new(),
        };
        let sev = if d.is_error() { "error" } else { "warning" };
        format!("{sev} [{}]{at} {}", d.code, d.message)
    };
    json!({
        "ok": r.ok,
        "diagnostics": r.diagnostics.iter().map(fmt).collect::<Vec<_>>(),
        "logs": r.logs,
        "slowestFrameMs": (r.frame_ms * 10.0).round() / 10.0,
        "size": format!("{}x{}", r.width, r.height),
    })
}

#[cfg(not(feature = "graphics"))]
pub fn check(_: Option<&ove_store::project::Project>, _: &Value, _: f64) -> Value {
    let _ = size_for(None);
    json!({"ok": false, "diagnostics": ["error [unavailable] this build cannot check motion graphics code"]})
}

fn check_text(v: &Value) -> String {
    let n = v["diagnostics"].as_array().map(Vec::len).unwrap_or(0);
    if n == 0 {
        format!("no problems (slowest frame {} ms)", v["slowestFrameMs"])
    } else {
        v["diagnostics"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
    }
}

pub fn check_code(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let c = a
        .value("composition")
        .filter(|c| c.is_object())
        .ok_or_else(|| Fail::new("argument.invalid", "`composition` must be an object"))?;
    let d = match a.value("duration") {
        None => 4.0,
        Some(v) => v
            .as_f64()
            .filter(|d| *d > 0.0 && d.is_finite())
            .ok_or_else(|| Fail::new("argument.invalid", "`duration` must be a positive number"))?,
    };
    // With a project (the session's, when there is one), assets and frame size come from it.
    let v = match ctx.bundle(a.project()) {
        Ok(b) => crate::ctx::read(ctx, &b, |p| Ok(check(Some(p), c, d)))?,
        Err(_) if a.project().is_none() => check(None, c, d),
        Err(e) => return Err(e),
    };
    let text = check_text(&v);
    Ok(Receipt::new(v, text))
}

pub fn ask_user(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("tool.unavailable", "ask_user is answered by the person in the app; it is not available here"))
}
