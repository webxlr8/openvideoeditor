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
    Play {
        /// Playback rate: 0.5, 1, 1.25, 1.5 or 2 (default: keep the current one).
        #[arg(long)]
        rate: Option<f64>,
    },
    /// Pause playback.
    Pause,
}

pub fn run(ctx: &Ctx, cmd: &AppCmd) -> Out {
    match cmd {
        AppCmd::Context => ctx.tool("app_context", json!({})),
        AppCmd::Open { project } => ctx.tool("open_in_app", json!({"project": project})),
        AppCmd::Seek { seconds } => ctx.tool("seek", json!({"seconds": seconds})),
        AppCmd::Play { rate } => ctx.tool("play", json!({"rate": rate})),
        AppCmd::Pause => ctx.tool("pause", json!({})),
    }
}
