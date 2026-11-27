//! `ove app …`: what the running app shows, and driving its transport.

use clap::Subcommand;
use serde_json::json;

use crate::common::{Ctx, Out};

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

pub fn run(ctx: &Ctx, cmd: &AppCmd) -> Out {
    match cmd {
        AppCmd::Context => ctx.tool("app_context", json!({})),
        AppCmd::Open { project } => ctx.tool("open_in_app", json!({"project": project})),
        AppCmd::Seek { seconds } => ctx.tool("seek", json!({"seconds": seconds})),
        AppCmd::Play => ctx.tool("play", json!({})),
        AppCmd::Pause => ctx.tool("pause", json!({})),
    }
}
