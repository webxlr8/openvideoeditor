//! `ove app …`: what the running app shows, and driving its transport.

use clap::Subcommand;
use serde_json::{Value, json};

use crate::common::{Ctx, Fail, Out};

#[derive(Subcommand)]
pub enum AppCmd {
    /// The open project, playhead, play state, selection and viewer mode of the running app.
    Context,
    /// Open a project in the app.
    Open { project: String },
    /// Move the playhead to a time in seconds (pauses).
    Seek { seconds: f64 },
    /// Start playback.
    Play,
    /// Pause playback.
    Pause,
}

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

pub fn run(ctx: &Ctx, cmd: &AppCmd) -> Out {
    match cmd {
        AppCmd::Context => {
            let v = ctx.app("context", json!({}))?;
            let text = context_text(&v);
            Ok((v, text))
        }
        AppCmd::Open { project } => {
            let bundle = ctx.find(project)?;
            let v = ctx.app("open", json!({"bundle": bundle}))?;
            Ok((v, format!("opened {} in the app", bundle.display())))
        }
        AppCmd::Seek { seconds } => {
            if !seconds.is_finite() || *seconds < 0.0 {
                return Err(Fail::new("argument.invalid", "seconds must be ≥ 0"));
            }
            let v = ctx.app("seek", json!({"seconds": seconds}))?;
            Ok((v, format!("playhead at {seconds:.3} s")))
        }
        AppCmd::Play => Ok((ctx.app("play", json!({}))?, "playing".into())),
        AppCmd::Pause => Ok((ctx.app("pause", json!({}))?, "paused".into())),
    }
}
